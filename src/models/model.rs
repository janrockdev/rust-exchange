pub mod models {
    use std::fmt;
    use ordered_float::OrderedFloat;
    use serde::{ Serialize, Serializer, ser::SerializeStruct, Deserialize, Deserializer };
    use uuid::Uuid;

    use colored::*;
    extern crate env_logger;
    extern crate log;

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

    // For Orderbook
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Order {
        pub id: Uuid,
        pub price: OrderedFloat<f64>,
        pub volume: OrderedFloat<f64>,
        pub side: String,
        pub timestamp: String,
        pub order_type: String,
    }

    // Custom deserialization for Order
    impl<'de> Deserialize<'de> for Order {
        fn deserialize<D>(deserializer: D) -> Result<Order, D::Error>
        where
            D: Deserializer<'de>,
        {
            #[derive(Deserialize)]
            struct OrderData {
                price: f64,
                volume: f64,
                side: String,
                timestamp: String,
                order_type: String,
            }

            let helper = OrderData::deserialize(deserializer)?;
            //let id = Uuid::parse_str(&helper.id).map_err(de::Error::custom)?;

            Ok(Order {
                id: Uuid::new_v4(),
                price: OrderedFloat(helper.price),
                volume: OrderedFloat(helper.volume),
                side: helper.side,
                timestamp: helper.timestamp,
                order_type: helper.order_type,
            })
        }
    }

    // Custom serialization for Order (to avoid serialization of OrderedFloat for csv crate)
    // TODO: find a better way to handle serialization of OrderedFloat
    impl Serialize for Order {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut state: <S as Serializer>::SerializeStruct =
                serializer.serialize_struct("Order", 5)?;
            state.serialize_field("price", &self.price.into_inner())?;
            state.serialize_field("volume", &self.volume.into_inner())?;
            state.serialize_field("side", &self.side)?;
            state.serialize_field("timestamp", &self.timestamp)?;
            state.serialize_field("order_type", &self.order_type)?;
            //state.serialize_field("id", &self.id.to_string())?;
            state.end()
        }
    }

    // Implement the Display trait for the Order struct (more readable output with colors)
    impl fmt::Display for Order {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let output: String = format!(
                "Price: {:.5}, Volume: {:.3}, Side: {}, ID: {}, Timestamp: {}",
                self.price,
                self.volume,
                self.side,
                self.id,
                self.timestamp
            );
            if self.side == "ask" {
                write!(f, "{}", output.red())
            } else {
                write!(f, "{}", output.green())
            }
        }
    }

    //For Tradebook
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Trade {
        pub id: Uuid,
        pub trader: String,
        pub pair: String,
        pub price: OrderedFloat<f64>,
        pub volume: OrderedFloat<f64>,
        pub side: String,
        pub timestamp: String,
        pub order_type: String,
        pub status: String,
    }
}
