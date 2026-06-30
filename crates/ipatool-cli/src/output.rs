use serde::Serialize;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

pub fn print_json<T: Serialize + ?Sized>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error serializing output: {e}"),
    }
}

pub fn print_apps(apps: &[ipatool_core::model::App], format: OutputFormat) {
    match format {
        OutputFormat::Text => {
            for app in apps {
                let price = if app.price == 0.0 {
                    "Free".to_string()
                } else {
                    format!("${:.2}", app.price)
                };
                println!(
                    "{} ({}) - {} [{}] {}",
                    app.name,
                    app.bundle_id,
                    app.version.as_deref().unwrap_or("?"),
                    app.id,
                    price
                );
            }
        }
        OutputFormat::Json => print_json(apps),
    }
}

#[derive(Serialize)]
struct AccountOutput<'a> {
    name: &'a str,
    email: &'a str,
    directory_services_id: &'a str,
    store_front: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pod: Option<&'a str>,
}

pub fn print_account(account: &ipatool_core::model::Account, format: OutputFormat) {
    match format {
        OutputFormat::Text => {
            println!("Name:     {}", account.name);
            println!("Email:    {}", account.email);
            println!("DSID:     {}", account.directory_services_id);
            println!("Store:    {}", account.store_front);
            if let Some(ref pod) = account.pod {
                println!("Pod:      {pod}");
            }
        }
        OutputFormat::Json => print_json(&AccountOutput {
            name: &account.name,
            email: &account.email,
            directory_services_id: &account.directory_services_id,
            store_front: &account.store_front,
            pod: account.pod.as_deref(),
        }),
    }
}
