use dotenv::dotenv;
use std::env;
use tracing::{ Level };

#[derive(Clone)]
pub struct EVMIndexerConfig {
    pub rpc_url: String,
    pub master_registry_contract: String,
    pub from_block: u64,
}

#[derive(Clone)]
pub struct LoggerConfig {
    pub logger_level: Level,
}

pub struct Config {
    pub evm_indexer_config: EVMIndexerConfig,
    pub logger_config: LoggerConfig,
}

fn parse_level_from_string(level: &str) -> Option<Level> {
    match level.to_lowercase().as_str() {
        "trace" => Some(Level::TRACE),
        "debug" => Some(Level::DEBUG),
        "info" => Some(Level::INFO),
        "warn" => Some(Level::WARN),
        "error" => Some(Level::ERROR),
        _ => None,
    }
}

impl Config {
    pub fn from_env() -> Self {
        dotenv().ok();

        let rpc_url = env
            ::var("CLIQUE_EVM_INDEXER_RPC_URL")
            .expect("CLIQUE_EVM_INDEXER_RPC_URL not found in .env");

        let from_block = env
            ::var("CLIQUE_EVM_INDEXER_FROM_BLOCK")
            .expect("CLIQUE_EVM_INDEXER_FROM_BLOCK not found in .env")
            .parse::<u64>().unwrap_or_else(|_| 0);

        let master_registry_contract = env
            ::var("CLIQUE_EVM_INDEXER_MASTER_REGISTRY_ADDRESS")
            .expect("CLIQUE_EVM_INDEXER_MASTER_REGISTRY_ADDRESS not found in .env");

        let logger_level_str = env::var("LOGGER_LEVEL").unwrap_or_else(|_| "info".to_string());
        let logger_level = parse_level_from_string(&logger_level_str).unwrap();

        let evm_indexer_config = EVMIndexerConfig {
            rpc_url,
            from_block,
            master_registry_contract,
        };

        let logger_config = LoggerConfig {
            logger_level,
        };

        Config {
            evm_indexer_config,
            logger_config,
        }
    }
}