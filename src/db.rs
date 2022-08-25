use std::io::Error;

use sled::Tree;

use crate::api::AuthenticatedUser;

pub struct BiedStore {
    accounts: Tree,
}

impl BiedStore {
    pub fn new(dir: &str) -> Self {
        let db = sled::open(dir).expect("failed to open database");
        Self {
            accounts: db.open_tree("accounts").expect("failed to create db tree"),
        }
    }
    
    pub fn insert_account(&mut self, title: &str, user: AuthenticatedUser) -> Result<(), Error> {
        self.accounts
            .insert(&title, bincode::serialize(&user).unwrap())?;
        Ok(())
    }

    pub fn fetch_accounts(&self) -> Vec<(String, AuthenticatedUser)> {
        self.accounts
            .range::<&str, _>(..)
            .filter_map(|e| e.ok())
            .filter_map(|d| {
                Some((
                    String::from_utf8(d.0.to_vec()).ok()?,
                    bincode::deserialize(&d.1).ok()?,
                ))
            })
            .collect() // TODO: return iterator instead
    }
}
