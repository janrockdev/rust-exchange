pub mod models {
    use serde::Deserialize;

    pub mod orderbook {
        tonic::include_proto!("orderbook");
    }

    #[derive(Debug, Deserialize)]
    pub struct Config {
        pub kraken: KrakenConfig,
    }

    #[derive(Debug, Deserialize)]
    pub struct KrakenConfig {
        pub symbols: Vec<String>,
        pub persist: String,
        pub offline: Vec<String>,
    }
}