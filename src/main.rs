use rand::prelude::*;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Element {
    pub result: String,
    pub emoji: String,
    pub is_new: bool,
}

type Nodes = BTreeMap<String, Element>;
type Pairs = BTreeMap<String, Option<String>>;
fn save(nodes: &Nodes, pairs: &Pairs) {
    let nodes_serialized = serde_json::to_string_pretty(nodes).unwrap();
    let pairs_serialized = serde_json::to_string_pretty(pairs).unwrap();
    let serialized_for_page = {
        let mut map = BTreeMap::new();
        map.insert(
            "elements",
            nodes
                .values()
                .map(|node| {
                    #[derive(Debug, Serialize)]
                    struct SerializedElement {
                        text: String,
                        emoji: String,
                        discovered: bool,
                    }
                    impl From<&Element> for SerializedElement {
                        fn from(value: &Element) -> Self {
                            SerializedElement {
                                text: value.result.clone(),
                                emoji: value.emoji.clone(),
                                discovered: value.is_new,
                            }
                        }
                    }

                    SerializedElement::from(node)
                })
                .collect::<Vec<_>>(),
        );
        serde_json::to_string(&map).unwrap()
    };
    std::fs::write("nodes.json", nodes_serialized).unwrap();
    std::fs::write("pairs.json", pairs_serialized).unwrap();
    std::fs::write("serialized_for_page.json", serialized_for_page).unwrap();
}
fn load() -> (Nodes, Pairs) {
    let nodes: Nodes =
        serde_json::from_str(&std::fs::read_to_string("nodes.json").unwrap()).unwrap();
    let pairs: Pairs =
        serde_json::from_str(&std::fs::read_to_string("pairs.json").unwrap()).unwrap();
    (nodes, pairs)
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

#[tokio::main]
async fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();
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
