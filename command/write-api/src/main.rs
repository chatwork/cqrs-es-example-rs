use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use anyhow::Result;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_dynamodb::config::{Credentials, Region};
use aws_sdk_dynamodb::Client;
use config::{Config, Environment};
use serde::Deserialize;

use cqrs_es_example_command_interface_adaptor::controllers::create_router;
use cqrs_es_example_command_interface_adaptor::gateways::event_persistence_gateway::EventPersistenceGateway;
use cqrs_es_example_command_interface_adaptor::gateways::thread_repository::ThreadRepositoryImpl;

#[derive(Deserialize, Debug)]
struct AppSettings {
  api: ApiSettings,
  persistence: PersistenceSettings,
  aws: AwsSettings,
}

#[derive(Deserialize, Debug)]
struct ApiSettings {
  host: String,
  port: u16,
}

#[derive(Deserialize, Debug)]
struct PersistenceSettings {
  journal_table_name: String,
  journal_aid_index_name: String,
  snapshot_table_name: String,
  snapshot_aid_index_name: String,
  journal_shard_count: u64,
}

#[derive(Deserialize, Debug)]
struct AwsSettings {
  region_name: String,
  endpoint_url: Option<String>,
  access_key_id: Option<String>,
  secret_access_key: Option<String>,
}

#[tokio::main]
async fn main() {
  tracing_subscriber::fmt().with_target(false).compact().init();

  let app_config = load_app_config().unwrap();
  let aws_client = create_aws_client(&app_config.aws).await;
  let socket_addr = SocketAddr::new(IpAddr::from_str(&app_config.api.host).unwrap(), app_config.api.port);
  let egg = EventPersistenceGateway::new(
    aws_client,
    app_config.persistence.journal_table_name.clone(),
    app_config.persistence.journal_aid_index_name.clone(),
    app_config.persistence.snapshot_table_name.clone(),
    app_config.persistence.snapshot_aid_index_name.clone(),
    app_config.persistence.journal_shard_count.clone(),
  );
  let repository = ThreadRepositoryImpl::new(egg);
  tracing::info!("Server listening on {}", socket_addr);
  axum::Server::bind(&socket_addr)
    .serve(create_router(repository).into_make_service())
    .await
    .unwrap();
}

fn load_app_config() -> Result<AppSettings> {
  let config = Config::builder()
    .add_source(config::File::with_name("config/write-api"))
    .add_source(Environment::with_prefix("WRITE_API"))
    .build()?;
  let app_config = config.try_deserialize()?;
  Ok(app_config)
}

async fn create_aws_client(aws_settings: &AwsSettings) -> Client {
  let region_name = aws_settings.region_name.clone();
  let region = Region::new(region_name);
  let region_provider_chain = RegionProviderChain::default_provider().or_else(region);
  let mut config_loader = aws_config::from_env().region(region_provider_chain);
  if let Some(endpoint_url) = aws_settings.endpoint_url.clone() {
    config_loader = config_loader.endpoint_url(endpoint_url);
  }
  match (
    aws_settings.access_key_id.clone(),
    aws_settings.secret_access_key.clone(),
  ) {
    (Some(access_key_id), Some(secret_access_key)) => {
      config_loader = config_loader.credentials_provider(Credentials::new(
        access_key_id,
        secret_access_key,
        None,
        None,
        "default",
      ));
    }
    _ => {}
  }
  let config = config_loader.load().await;
  let client = Client::new(&config);
  client
}

// use axum::routing::get;
// use axum::Router;
// use std::net::SocketAddr;
//
// async fn hello_write_api() -> &'static str {
//   "Hello, Write API!"
// }
//
// #[tokio::main]
// async fn main() {
//   tracing_subscriber::fmt().with_target(false).compact().init();
//   let socket_addr = SocketAddr::from(([0, 0, 0, 0], 8080));
//   tracing::info!("Server listening on {}", socket_addr);
//   axum::Server::bind(&socket_addr)
//     .serve(Router::new().route("/", get(hello_write_api)).into_make_service())
//     .await
//     .unwrap();
// }
