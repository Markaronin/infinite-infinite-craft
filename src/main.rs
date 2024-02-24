use clap::{Parser, Subcommand};
use rand::{distributions::WeightedIndex, prelude::*};
use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Doc comment
#[derive(Debug, Subcommand)]
enum Command {
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

    SerializeForPage,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Element {
    pub result: String,
    pub emoji: String,
    pub is_new: bool,
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

type Elements = BTreeMap<String, Element>;
type Pairs = BTreeMap<String, Option<String>>;

fn load() -> (Elements, Pairs) {
    let elements: Elements = read_file_as_json("elements.json");
    let pairs: Pairs = read_file_as_json("pairs.json");
    (elements, pairs)
}

fn save(elements: &Elements, pairs: &Pairs) {
    write_file_as_json("elements.json", elements, true);
    write_file_as_json("pairs.json", pairs, true);
}

fn serialize_for_page() {
    let (elements, _) = load();

    let elements = SerializedElements {
        elements: elements
            .values()
            .cloned()
            .map(|element| SerializedElement::from(element))
            .collect::<Vec<_>>(),
    };
    write_file_as_json("serialized_for_page.json", &elements, false);
}

async fn get_pair_value(first: &str, second: &str) -> Option<Element> {
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:122.0) Gecko/20100101 Firefox/122.0",
        )
        .http1_title_case_headers()
        .build()
        .unwrap();

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
        if element.result == "Nothing" {
            None
        } else {
            Some(element)
        }
    }
}

async fn do_combinations() {
    let mut rng = thread_rng();

    let (mut elements, mut pairs) = load();

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
                format!("{first}|+|{second}")
            } else {
                format!("{second}|+|{first}")
            };

            if !pairs.contains_key(&pair_key) {
                break (first, second, pair_key);
            }
        };

        let pair_result = get_pair_value(first, second).await;
        pairs.insert(pair_key.clone(), pair_result.clone().map(|p| p.result));
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
                elements.insert(pair_result.result.clone(), pair_result);
            }
        }

        save(&elements, &pairs);

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn merge_existing_elements(elements_file_path: &str) {
    let (mut elements, pairs) = load();

    let new_elements: SerializedElements = read_file_as_json(elements_file_path);

    for element in new_elements
        .elements
        .into_iter()
        .map(|element| Element::from(element))
    {
        match elements.entry(element.result.clone()) {
            std::collections::btree_map::Entry::Vacant(vacant_entry) => {
                log::info!("Inserting {}", element.result);
                vacant_entry.insert(element);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &element {
                    panic!(
                        "Non-matching elements despite matching names\n{:?}\n{:?}",
                        entry.get(),
                        element
                    )
                }
            }
        }
    }

    save(&elements, &pairs)
}

#[tokio::main]
async fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();

    let cli = Cli::parse();

    match cli.command {
        Command::Combine => do_combinations().await,
        Command::MergeExistingElements { elements_file_path } => {
            merge_existing_elements(&elements_file_path)
        }
        Command::SerializeForPage => serialize_for_page(),
    }
}
