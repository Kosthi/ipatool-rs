use std::collections::HashMap;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

pub async fn purchase(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
) -> Result<(), ClientError> {
    match try_purchase(client, app_id, account, "STDQ").await {
        Err(ClientError::Store(StoreError::TemporarilyUnavailable)) => {
            tracing::info!("STDQ unavailable, trying GAME pricing");
            try_purchase(client, app_id, account, "GAME").await
        }
        other => other,
    }
}

async fn try_purchase(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    pricing_parameters: &str,
) -> Result<(), ClientError> {
    let url = buy_url(account);

    let mut body = plist::Dictionary::new();
    body.insert(
        "appExtVrsId".into(),
        plist::Value::String("0".into()),
    );
    body.insert(
        "hasAskedToFulfillPreorder".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "buyWithoutAuthorization".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "hasBeenAuthedForBuy".into(),
        plist::Value::String("true".into()),
    );
    body.insert(
        "guid".into(),
        plist::Value::String(client.guid().to_string()),
    );
    body.insert(
        "needDiv".into(),
        plist::Value::String("0".into()),
    );
    body.insert(
        "origPage".into(),
        plist::Value::String(format!("Software-{app_id}")),
    );
    body.insert(
        "origPageLocation".into(),
        plist::Value::String("Buy".into()),
    );
    body.insert(
        "price".into(),
        plist::Value::String("0".into()),
    );
    body.insert(
        "pricingParameters".into(),
        plist::Value::String(pricing_parameters.into()),
    );
    body.insert(
        "productType".into(),
        plist::Value::String("C".into()),
    );
    body.insert(
        "salableAdamId".into(),
        plist::Value::String(app_id.to_string()),
    );

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

    let resp = client
        .http()
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("iCloud-DSID", &account.directory_services_id)
        .header("X-Dsid", &account.directory_services_id)
        .header("X-Apple-Store-Front", &account.store_front)
        .header("X-Token", &account.password_token)
        .body(body_bytes)
        .send()
        .await?;

    let resp_body = resp.bytes().await?;
    let dict: HashMap<String, plist::Value> =
        crate::client::plist_xml::parse_plist_response(&resp_body)?;

    if let Some(err) = StoreError::from_plist_dict(&dict) {
        match err {
            StoreError::LicenseAlreadyExists => {
                tracing::info!("license already exists");
                return Ok(());
            }
            other => return Err(ClientError::Store(other)),
        }
    }

    Ok(())
}

fn buy_url(account: &Account) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/buyProduct")
}
