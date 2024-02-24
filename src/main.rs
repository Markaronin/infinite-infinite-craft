use clap::{Parser, Subcommand};
use rand::prelude::*;
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
    MergeExistingNodes {
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

type Nodes = BTreeMap<String, Element>;
type Pairs = BTreeMap<String, Option<String>>;

fn load() -> (Nodes, Pairs) {
    let nodes: Nodes = read_file_as_json("nodes.json");
    let pairs: Pairs = read_file_as_json("pairs.json");
    (nodes, pairs)
}

fn save(nodes: &Nodes, pairs: &Pairs) {
    write_file_as_json("nodes.json", nodes, true);
    write_file_as_json("pairs.json", pairs, true);
}

fn serialize_for_page() {
    let (nodes, _) = load();

    let elements = SerializedElements {
        elements: nodes
            .values()
            .cloned()
            .map(|node| SerializedElement::from(node))
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

    let (mut nodes, mut pairs) = load();

    loop {
        let index_1 = rng.gen_range(0..nodes.len());
        let index_2 = rng.gen_range(0..nodes.len());

        let first = nodes.keys().nth(index_1).unwrap();
        let second = nodes.keys().nth(index_2).unwrap();

        // Sort pairs so that we don't make the same query twice
        let pair_key = if first < second {
            format!("{first}|+|{second}")
        } else {
            format!("{second}|+|{first}")
        };

        if !pairs.contains_key(&pair_key) {
            let pair_result = get_pair_value(first, second).await;
            pairs.insert(pair_key.clone(), pair_result.clone().map(|p| p.result));
            if let Some(pair_result) = pair_result {
                if !nodes.contains_key(&pair_result.result) {
                    if pair_result.is_new {
                        log::info!(
                            "Discovered new node: {} (from {first} and {second})",
                            pair_result.result
                        );
                    } else {
                        log::info!(
                            "New node: {} (from {first} and {second})",
                            pair_result.result
                        );
                    }
                    nodes.insert(pair_result.result.clone(), pair_result);
                }
            }

            save(&nodes, &pairs);

            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

fn merge_existing_nodes(elements_file_path: &str) {
    let (mut nodes, pairs) = load();

    let new_elements: SerializedElements = read_file_as_json(elements_file_path);

    for element in new_elements
        .elements
        .into_iter()
        .map(|element| Element::from(element))
    {
        match nodes.entry(element.result.clone()) {
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

    save(&nodes, &pairs)
}

#[tokio::main]
async fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();

    let cli = Cli::parse();

    match cli.command {
        Command::Combine => do_combinations().await,
        Command::MergeExistingNodes { elements_file_path } => {
            merge_existing_nodes(&elements_file_path)
        }
        Command::SerializeForPage => serialize_for_page(),
    }
}
