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

    pub fn insert_account(
        &mut self,
        title: &str,
        user: AuthenticatedUser,
    ) -> Result<(), StoreError> {
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

    pub fn fetch_account(&self, title: &str) -> Result<AuthenticatedUser, StoreError> {
        Ok(bincode::deserialize(&self.accounts.get(title)?.ok_or(
            StoreError("no account with that name".to_string()),
        )?)?)
    }

    pub fn remove_account(&mut self, title: &str) -> Result<AuthenticatedUser, StoreError> {
        self.accounts
            .remove(title)?
            .ok_or(StoreError("No account with that name".to_string()))
            .map(|e| bincode::deserialize::<AuthenticatedUser>(&e).map_err(|e| e.into()))?
    }

    pub fn rename_account(&mut self, old: &str, new: &str) -> Result<(), StoreError> {
        self.accounts.insert(
            new,
            self.accounts
                .remove(old)?
                .ok_or(StoreError("no account with that name".to_string()))?,
        )?;
        Ok(())
    }
}

// TODO: use errors based on an enum, not a string
#[derive(Debug)]
pub struct StoreError(String);

impl From<sled::Error> for StoreError {
    fn from(e: sled::Error) -> Self {
        Self(format!("{:?}", e))
    }
}

impl From<bincode::Error> for StoreError {
    fn from(e: bincode::Error) -> Self {
        Self(format!("{:?}", e))
    }
}
