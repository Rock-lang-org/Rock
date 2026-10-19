use std::collections::HashMap;

use super::TokenStream;
#[derive(Debug, Clone)]
pub struct Correspondance {
    pub entries: HashMap<String, Vec<TokenStream>>,
    pub nested_corresp_keys: Vec<Vec<String>>,
    pub nested_corresp: Vec<Correspondance>,
}

impl Correspondance {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            nested_corresp_keys: vec![],
            nested_corresp: vec![],
        }
    }

    pub fn insert_direct(&mut self, name: String, tokens: TokenStream) {
        let entry = self.entries.entry(name).or_insert(vec![tokens.clone()]);

        if !entry.contains(&tokens) {
            entry.push(tokens);
        }
    }

    pub fn insert_nested(&mut self, correspondance: Correspondance) {
        if self
            .nested_corresp_keys
            .contains(&correspondance.keys().clone())
        {
            let nested_idx = self
                .nested_corresp_keys
                .iter()
                .enumerate()
                .find(|(_i, names)| **names == correspondance.keys())
                .map(|(i, _)| i);

            if let Some(i) = nested_idx {
                self.nested_corresp[i].merge(&correspondance);
            }
        } else {
            self.nested_corresp_keys.push(correspondance.keys());
            self.nested_corresp.push(correspondance);
        };
    }

    pub fn keys(&self) -> Vec<String> {
        self.entries
            .keys()
            .cloned()
            .chain(
                self.nested_corresp
                    .iter()
                    .flat_map(|corresp| corresp.keys()),
            )
            .collect()
    }

    pub fn get(&self, name: &str, max_level: usize) -> Option<Vec<TokenStream>> {
        if max_level == 0 {
            return self.entries.get(name).cloned();
        } else {
            if let Some(tokens) = self.entries.get(name) {
                return Some(tokens.clone());
            }

            for (i, inner_name) in self.nested_corresp_keys.iter().enumerate() {
                if inner_name.contains(&name.to_string()) {
                    return self.nested_corresp[i].get(name, max_level - 1);
                }
            }
            None
        }
    }

    pub fn merge(&mut self, other: &Self) {
        for (name, tokens) in other.entries.clone() {
            let entry = self.entries.entry(name).or_default();

            entry.extend(tokens);
        }

        for nested_corresp in other.nested_corresp.iter() {
            if self.nested_corresp_keys.contains(&nested_corresp.keys()) {
                let idx = self
                    .nested_corresp_keys
                    .iter()
                    .position(|keys| *keys == nested_corresp.keys())
                    .unwrap();

                self.nested_corresp[idx].merge(&nested_corresp.clone());
            } else {
                self.nested_corresp_keys.push(nested_corresp.keys());
                self.nested_corresp.push(nested_corresp.clone());
            }
        }
    }
}
