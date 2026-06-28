use std::path::Path;
use std::sync::Arc;

use cookie_store::CookieStore;
use reqwest_cookie_store::CookieStoreMutex;

use crate::error::ClientError;

pub fn new_cookie_store(path: Option<&Path>) -> Result<Arc<CookieStoreMutex>, ClientError> {
    let store = if let Some(p) = path {
        if p.exists() {
            let file = std::fs::File::open(p)
                .map_err(|e| ClientError::UnexpectedResponse(format!("cookie file: {e}")))?;
            let reader = std::io::BufReader::new(file);
            #[allow(deprecated)]
            CookieStore::load_json(reader)
                .map_err(|e| ClientError::UnexpectedResponse(format!("cookie parse: {e}")))?
        } else {
            CookieStore::default()
        }
    } else {
        CookieStore::default()
    };

    Ok(Arc::new(CookieStoreMutex::new(store)))
}

pub fn save_cookie_store(store: &CookieStoreMutex, path: &Path) -> Result<(), ClientError> {
    let mut file = std::fs::File::create(path)
        .map_err(|e| ClientError::UnexpectedResponse(format!("cookie file create: {e}")))?;
    let guard = store.lock().unwrap();
    #[allow(deprecated)]
    guard
        .save_json(&mut file)
        .map_err(|e| ClientError::UnexpectedResponse(format!("cookie save: {e}")))?;
    Ok(())
}
