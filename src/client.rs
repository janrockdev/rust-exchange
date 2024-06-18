use orderbook::order_book_client::OrderBookClient;
use orderbook::OrderRequest;
use structopt::StructOpt;

pub mod orderbook {
    tonic::include_proto!("orderbook");
}

#[derive(StructOpt, Debug)]
#[structopt(name = "TradingCLI", about = "A CLI to submit market and limit trades.")]
struct Cli {
    #[structopt(subcommand)]
    command: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    MarketOrder {
        pair: String,
        volume: f64,
        side: String,
        order_type: String,
        price: f64,
        trader: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut client = OrderBookClient::connect("http://[::1]:50051").await?;
    println!("Connected to Server.");

    let args = Cli::from_args();

    match args.command {
        Command::MarketOrder { pair, volume, side, order_type, price, trader } => {
            let market_order_request = tonic::Request::new(OrderRequest {
                pair,
                volume,
                side,
                order_type,
                price,
                trader,
            });
            let response = client.place_market_order(market_order_request).await?;
            println!("Order Response: {:?}", response.into_inner());
        },
    }

    Ok(())
}
