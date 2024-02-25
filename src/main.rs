use clap::{Parser, Subcommand};
use rand::{distributions::WeightedIndex, prelude::*};
use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::{prelude::FromRow, SqlitePool};
use std::{collections::BTreeMap, time::Duration, time::Instant};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Doc comment
#[derive(Debug, Subcommand)]
enum Command {
    /// Run random combinations every 0.5ish seconds to create new elements
    Combine,

    /// Meant to import your existing save from the website into the list of elements in this repo
    ///
    /// Prepare the elements file by copying the "infinite-craft-data" value from localstorage for https://neal.fun/infinite-craft/
    /// and pasting it into a file, then pass that file path to this command.
    /// The file should look something like {"elements": [{"text": "Water", "emoji":"💧", discovered: false }, ...]}
    MergeExistingElements {
        #[arg(short, long)]
        elements_file_path: String,
    },

    /// Export the data in a way that you can copy into your localstorage and interact with
    SerializeForPage,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct Element {
    pub result: String,
    pub emoji: String,
    pub is_new: bool,
}
impl Element {
    pub async fn insert(&self, pool: &SqlitePool) {
        sqlx::query("INSERT INTO elements (result, emoji, is_new) VALUES ($1, $2, $3)")
            .bind(&self.result)
            .bind(&self.emoji)
            .bind(self.is_new)
            .execute(pool)
            .await
            .unwrap();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SerializedElements {
    elements: Vec<SerializedElement>,
}
/**
 *  The element as it appears in localstorage on the website
 */
#[derive(Debug, Serialize, Deserialize)]
struct SerializedElement {
    text: String,
    emoji: String,
    discovered: bool,
}
impl From<Element> for SerializedElement {
    fn from(value: Element) -> Self {
        SerializedElement {
            text: value.result,
            emoji: value.emoji,
            discovered: value.is_new,
        }
    }
}
impl From<SerializedElement> for Element {
    fn from(value: SerializedElement) -> Self {
        Element {
            result: value.text,
            emoji: value.emoji,
            is_new: value.discovered,
        }
    }
}

fn read_file_as_json<T>(file_path: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(&std::fs::read_to_string(file_path).unwrap()).unwrap()
}
fn write_file_as_json<T>(file_path: &str, contents: &T, pretty: bool)
where
    T: Serialize,
{
    let contents = if pretty {
        serde_json::to_string_pretty(contents)
    } else {
        serde_json::to_string(contents)
    }
    .unwrap();
    std::fs::write(file_path, contents).unwrap();
}

async fn insert_pair(pool: &SqlitePool, first: &str, second: &str, result: &Option<String>) {
    sqlx::query("INSERT INTO pairs (first, second, result) VALUES ($1, $2, $3)")
        .bind(first)
        .bind(second)
        .bind(result)
        .execute(pool)
        .await
        .unwrap();
}

type Elements = BTreeMap<String, Element>;
type Pairs = BTreeMap<(String, String), Option<String>>;

async fn load(pool: &SqlitePool) -> (Elements, Pairs) {
    let elements = sqlx::query_as::<_, Element>("SELECT * FROM elements")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|element| (element.result.clone(), element))
        .collect::<Elements>();

    let pairs = sqlx::query_as::<_, (String, String, Option<String>)>("SELECT * FROM pairs")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(first, second, result)| ((first, second), result))
        .collect::<Pairs>();

    (elements, pairs)
}

async fn serialize_for_page(pool: SqlitePool) {
    let (elements, _) = load(&pool).await;

    let elements = SerializedElements {
        elements: elements
            .values()
            .cloned()
            .map(SerializedElement::from)
            .collect::<Vec<_>>(),
    };
    write_file_as_json("serialized_for_page.json", &elements, false);
}

async fn get_pair_value(client: &reqwest::Client, first: &str, second: &str) -> Option<Element> {
    let start = Instant::now();

    let response = client
        .get(format!(
            "https://neal.fun/api/infinite-craft/pair?first={first}&second={second}"
        ))
        .header("Referer", "https://neal.fun/infinite-craft/")
        .send()
        .await
        .unwrap();

    if response.status() != StatusCode::OK {
        println!("Non-200 status code {response:#?}");
        panic!("{}", response.text().await.unwrap())
    } else {
        let element: Element = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        let response = if element.result == "Nothing" {
            None
        } else {
            Some(element)
        };

        log::debug!("Request took {} milliseconds", start.elapsed().as_millis());

        response
    }
}

async fn do_combinations(pool: SqlitePool) {
    let mut rng = thread_rng();

    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:122.0) Gecko/20100101 Firefox/122.0",
        )
        .http1_title_case_headers()
        .build()
        .unwrap();

    let (mut elements, mut pairs) = load(&pool).await;

    loop {
        // Weight it towards shorter objects - an element with 1 letter is ~5x more likely to show up than an element with 10+ letters
        let distribution =
            WeightedIndex::new(elements.keys().map(|element| 12 - element.len().min(10))).unwrap();

        let (first, second, pair_key) = loop {
            let index_1 = distribution.sample(&mut rng);
            let index_2 = distribution.sample(&mut rng);

            let first = elements.keys().nth(index_1).unwrap();
            let second = elements.keys().nth(index_2).unwrap();

            // Sort pairs so that we don't make the same query twice
            let pair_key = if first < second {
                (first.clone(), second.clone())
            } else {
                (second.clone(), first.clone())
            };

            if !pairs.contains_key(&pair_key) {
                break (first, second, pair_key);
            }
        };

        let pair_result = get_pair_value(&client, first, second).await;

        // These two statements have to happen together - do not remove or change one without the other
        pairs.insert(pair_key.clone(), pair_result.clone().map(|p| p.result));
        insert_pair(
            &pool,
            first,
            second,
            &pair_result.as_ref().map(|element| element.result.clone()),
        )
        .await;

        if let Some(pair_result) = pair_result {
            if !elements.contains_key(&pair_result.result) {
                if pair_result.is_new {
                    log::info!(
                        "Discovered new element: {} (from {first} and {second})",
                        pair_result.result
                    );
                } else {
                    log::info!(
                        "New element: {} (from {first} and {second})",
                        pair_result.result
                    );
                }

                // These two statements have to happen together - do not remove or change one without the other
                pair_result.insert(&pool).await;
                elements.insert(pair_result.result.clone(), pair_result);
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

async fn merge_existing_elements(pool: SqlitePool, elements_file_path: &str) {
    let new_elements: SerializedElements = read_file_as_json(elements_file_path);

    for element in new_elements.elements.into_iter().map(Element::from) {
        if let Some(matching_element) =
            sqlx::query_as::<_, Element>("SELECT * FROM elements WHERE result = $1")
                .bind(&element.result)
                .fetch_optional(&pool)
                .await
                .unwrap()
        {
            if matching_element != element {
                panic!(
                    "Non-matching elements despite matching names\n{:?}\n{:?}",
                    matching_element, element
                )
            }
        } else {
            log::info!("Inserting {}", element.result);
            element.insert(&pool).await;
        }
    }
}

#[tokio::main]
async fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();

    let pool = SqlitePool::connect("sqlite:infinite-craft.db")
        .await
        .unwrap();

    let cli = Cli::parse();

    match cli.command {
        Command::Combine => do_combinations(pool).await,
        Command::MergeExistingElements { elements_file_path } => {
            merge_existing_elements(pool, &elements_file_path).await
        }
        Command::SerializeForPage => serialize_for_page(pool).await,
    }
}
