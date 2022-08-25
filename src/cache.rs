use chrono::{Datelike, Utc};

use crate::{
    api::{ApiError, BiedApi, Offer},
    db::BiedStore,
};

// TODO: move cashe to file
pub struct BiedCache {
    pub offers: Vec<Offer>,
    collect_day: u32,
}

impl BiedCache {
    pub fn new() -> Self {
        Self {
            offers: vec![],
            collect_day: u32::MAX,
        }
    }

    // TODO: auto sync every day
    pub async fn sync_offers(
        &mut self,
        store: &mut BiedStore,
        api: &BiedApi,
    ) -> Result<(), ApiError> {
        if Utc::now().day() == self.collect_day {
            return Ok(());
        }
        self.offers.clear();
        for user in store.fetch_accounts() {
            for mut of in api.get_offers(user.1.auth).await {
                self.offers.append(&mut of);
            }
        }
        Ok(())
    }
}
