use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};
use tokio::sync::mpsc;

use orderbook::order_book_client::OrderBookClient;
use orderbook::{OrderBookRequest, OrderRequest, TradeBookRequest};

pub mod orderbook {
    tonic::include_proto!("orderbook");
}

const PAIRS: &[&str] = &["XXBTZUSD", "XETHZUSD", "SUIUSD"];

// ---------------------------------------------------------------------------
// App messages
// ---------------------------------------------------------------------------

enum AppMsg {
    OrderBookLoaded(Vec<(f64, f64)>),
    TradesLoaded(Vec<orderbook::Trade>),
    OrderResult(String),
    Err(String),
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
enum Tab {
    OrderBook,
    TradeBook,
    PlaceOrder,
}

struct OrderForm {
    pair_idx: usize,
    side_idx: usize,  // 0=buy  1=sell
    type_idx: usize,  // 0=market  1=limit
    price: String,
    volume: String,
    trader: String,
    field: usize, // 0=pair 1=side 2=type 3=price 4=volume 5=trader
}

impl Default for OrderForm {
    fn default() -> Self {
        Self {
            pair_idx: 0,
            side_idx: 0,
            type_idx: 0,
            price: String::new(),
            volume: String::new(),
            trader: String::new(),
            field: 0,
        }
    }
}

struct App {
    tab: Tab,
    // Order Book tab
    pair_idx: usize,
    order_book: Vec<(f64, f64)>,
    // Trade Book tab
    trader_input: String,
    trader_editing: bool,
    trades: Vec<orderbook::Trade>,
    // Place Order tab
    form: OrderForm,
    // Status bar
    status: String,
    status_err: bool,
    // Sender for async results
    msg_tx: mpsc::Sender<AppMsg>,
}

impl App {
    fn new(msg_tx: mpsc::Sender<AppMsg>) -> Self {
        Self {
            tab: Tab::OrderBook,
            pair_idx: 0,
            order_book: Vec::new(),
            trader_input: String::new(),
            trader_editing: false,
            trades: Vec::new(),
            form: OrderForm::default(),
            status: "Connecting...".to_string(),
            status_err: false,
            msg_tx,
        }
    }
}

// ---------------------------------------------------------------------------
// gRPC helpers (fire-and-forget, results returned via channel)
// ---------------------------------------------------------------------------

fn fetch_order_book(
    mut client: OrderBookClient<tonic::transport::Channel>,
    pair: String,
    tx: mpsc::Sender<AppMsg>,
) {
    tokio::spawn(async move {
        let req = tonic::Request::new(OrderBookRequest { pair });
        match client.get_order_book(req).await {
            Ok(r) => {
                let orders = r
                    .into_inner()
                    .orders
                    .into_iter()
                    .map(|o| (o.price, o.volume))
                    .collect();
                let _ = tx.send(AppMsg::OrderBookLoaded(orders)).await;
            }
            Err(e) => {
                let _ = tx.send(AppMsg::Err(e.to_string())).await;
            }
        }
    });
}

fn fetch_trades(
    mut client: OrderBookClient<tonic::transport::Channel>,
    trader: String,
    tx: mpsc::Sender<AppMsg>,
) {
    tokio::spawn(async move {
        let req = tonic::Request::new(TradeBookRequest { trader });
        match client.get_trade_book(req).await {
            Ok(r) => {
                let _ = tx.send(AppMsg::TradesLoaded(r.into_inner().trades)).await;
            }
            Err(e) => {
                let _ = tx.send(AppMsg::Err(e.to_string())).await;
            }
        }
    });
}

fn submit_order(
    mut client: OrderBookClient<tonic::transport::Channel>,
    req: OrderRequest,
    tx: mpsc::Sender<AppMsg>,
) {
    tokio::spawn(async move {
        match client.place_market_order(tonic::Request::new(req)).await {
            Ok(r) => {
                let inner = r.into_inner();
                let _ = tx
                    .send(AppMsg::OrderResult(format!(
                        "[{}] {}",
                        inner.status, inner.message
                    )))
                    .await;
            }
            Err(e) => {
                let _ = tx.send(AppMsg::Err(e.to_string())).await;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal).await;

    // Always restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(ref e) = result {
        eprintln!("Error: {}", e);
    }
    result
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = OrderBookClient::connect("http://[::1]:50051").await?;

    let (msg_tx, mut msg_rx) = mpsc::channel::<AppMsg>(64);
    let mut app = App::new(msg_tx.clone());

    fetch_order_book(client.clone(), PAIRS[app.pair_idx].to_string(), msg_tx.clone());
    app.status = "Connected — loading order book...".to_string();

    // Blocking event reader on a dedicated thread
    let (evt_tx, mut evt_rx) = mpsc::channel::<Event>(64);
    tokio::task::spawn_blocking(move || loop {
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => match event::read() {
                Ok(e) => {
                    if evt_tx.blocking_send(e).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
    });

    let mut refresh = tokio::time::interval(Duration::from_secs(10));
    refresh.tick().await; // consume the immediate first tick

    loop {
        terminal.draw(|f| ui(f, &app))?;

        tokio::select! {
            Some(evt) = evt_rx.recv() => {
                if let Event::Key(key) = evt {
                    if key.kind == KeyEventKind::Press
                        && handle_key(&mut app, key.code, &client)
                    {
                        break;
                    }
                }
            }
            Some(msg) = msg_rx.recv() => {
                match msg {
                    AppMsg::OrderBookLoaded(orders) => {
                        app.order_book = orders;
                        app.status = format!("Order book loaded — {}", PAIRS[app.pair_idx]);
                        app.status_err = false;
                    }
                    AppMsg::TradesLoaded(trades) => {
                        let n = trades.len();
                        app.trades = trades;
                        app.status = format!("{} trades loaded for '{}'", n, app.trader_input);
                        app.status_err = false;
                    }
                    AppMsg::OrderResult(msg) => {
                        app.status = msg;
                        app.status_err = false;
                    }
                    AppMsg::Err(e) => {
                        app.status = format!("Error: {}", e);
                        app.status_err = true;
                    }
                }
            }
            _ = refresh.tick() => {
                if app.tab == Tab::OrderBook {
                    fetch_order_book(
                        client.clone(),
                        PAIRS[app.pair_idx].to_string(),
                        msg_tx.clone(),
                    );
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Returns `true` when the app should quit.
fn handle_key(
    app: &mut App,
    code: KeyCode,
    client: &OrderBookClient<tonic::transport::Channel>,
) -> bool {
    let typing = is_typing(app);

    if !typing && matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
        return true;
    }

    if !typing {
        match code {
            KeyCode::Tab => {
                app.tab = match app.tab {
                    Tab::OrderBook => Tab::TradeBook,
                    Tab::TradeBook => Tab::PlaceOrder,
                    Tab::PlaceOrder => Tab::OrderBook,
                };
                return false;
            }
            KeyCode::BackTab => {
                app.tab = match app.tab {
                    Tab::OrderBook => Tab::PlaceOrder,
                    Tab::TradeBook => Tab::OrderBook,
                    Tab::PlaceOrder => Tab::TradeBook,
                };
                return false;
            }
            _ => {}
        }
    }

    match app.tab {
        Tab::OrderBook => handle_order_book_key(app, code, client),
        Tab::TradeBook => handle_trade_book_key(app, code, client),
        Tab::PlaceOrder => handle_place_order_key(app, code, client),
    }

    false
}

fn is_typing(app: &App) -> bool {
    match app.tab {
        Tab::TradeBook => app.trader_editing,
        Tab::PlaceOrder => app.form.field >= 3,
        _ => false,
    }
}

fn handle_order_book_key(
    app: &mut App,
    code: KeyCode,
    client: &OrderBookClient<tonic::transport::Channel>,
) {
    match code {
        KeyCode::Left | KeyCode::Char('h') => {
            if app.pair_idx > 0 {
                app.pair_idx -= 1;
                fetch_order_book(
                    client.clone(),
                    PAIRS[app.pair_idx].to_string(),
                    app.msg_tx.clone(),
                );
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if app.pair_idx < PAIRS.len() - 1 {
                app.pair_idx += 1;
                fetch_order_book(
                    client.clone(),
                    PAIRS[app.pair_idx].to_string(),
                    app.msg_tx.clone(),
                );
            }
        }
        KeyCode::Char('r') => {
            fetch_order_book(
                client.clone(),
                PAIRS[app.pair_idx].to_string(),
                app.msg_tx.clone(),
            );
        }
        _ => {}
    }
}

fn handle_trade_book_key(
    app: &mut App,
    code: KeyCode,
    client: &OrderBookClient<tonic::transport::Channel>,
) {
    if app.trader_editing {
        match code {
            KeyCode::Enter => {
                app.trader_editing = false;
                let trader = app.trader_input.clone();
                fetch_trades(client.clone(), trader, app.msg_tx.clone());
            }
            KeyCode::Esc => {
                app.trader_editing = false;
            }
            KeyCode::Char(c) => {
                app.trader_input.push(c);
            }
            KeyCode::Backspace => {
                app.trader_input.pop();
            }
            _ => {}
        }
    } else {
        match code {
            KeyCode::Char('i') | KeyCode::Enter => {
                app.trader_editing = true;
            }
            _ => {}
        }
    }
}

fn handle_place_order_key(
    app: &mut App,
    code: KeyCode,
    client: &OrderBookClient<tonic::transport::Channel>,
) {
    let in_text = app.form.field >= 3;

    match code {
        KeyCode::Up => {
            if app.form.field > 0 {
                app.form.field -= 1;
            }
        }
        KeyCode::Down => {
            if app.form.field < 5 {
                app.form.field += 1;
            }
        }
        KeyCode::Char('k') if !in_text => {
            if app.form.field > 0 {
                app.form.field -= 1;
            }
        }
        KeyCode::Char('j') if !in_text => {
            if app.form.field < 5 {
                app.form.field += 1;
            }
        }
        KeyCode::Left | KeyCode::Char('h') if !in_text => match app.form.field {
            0 => {
                if app.form.pair_idx > 0 {
                    app.form.pair_idx -= 1;
                }
            }
            1 => app.form.side_idx = 0,
            2 => app.form.type_idx = 0,
            _ => {}
        },
        KeyCode::Right | KeyCode::Char('l') if !in_text => match app.form.field {
            0 => {
                if app.form.pair_idx < PAIRS.len() - 1 {
                    app.form.pair_idx += 1;
                }
            }
            1 => app.form.side_idx = 1,
            2 => app.form.type_idx = 1,
            _ => {}
        },
        KeyCode::Char(c) if in_text => match app.form.field {
            3 => app.form.price.push(c),
            4 => app.form.volume.push(c),
            5 => app.form.trader.push(c),
            _ => {}
        },
        KeyCode::Backspace if in_text => match app.form.field {
            3 => {
                app.form.price.pop();
            }
            4 => {
                app.form.volume.pop();
            }
            5 => {
                app.form.trader.pop();
            }
            _ => {}
        },
        KeyCode::Enter => {
            if app.form.field == 5 {
                let pair = PAIRS[app.form.pair_idx].to_string();
                let volume_str = app.form.volume.clone();
                let price_str = app.form.price.clone();
                let side = if app.form.side_idx == 0 { "buy" } else { "sell" };
                let order_type = if app.form.type_idx == 0 { "market" } else { "limit" };
                let trader = app.form.trader.clone();
                let tx = app.msg_tx.clone();

                match volume_str.parse::<f64>() {
                    Ok(volume) => {
                        let price = price_str.parse::<f64>().unwrap_or(0.0);
                        let req = OrderRequest {
                            pair,
                            volume,
                            side: side.to_string(),
                            order_type: order_type.to_string(),
                            price,
                            trader,
                        };
                        submit_order(client.clone(), req, tx);
                        app.status = "Submitting order...".to_string();
                        app.status_err = false;
                    }
                    Err(_) => {
                        app.status =
                            "Invalid volume — must be a number (e.g. 0.01)".to_string();
                        app.status_err = true;
                    }
                }
            } else {
                app.form.field += 1;
            }
        }
        KeyCode::Esc => {
            app.form.field = 0;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// UI rendering
// ---------------------------------------------------------------------------

fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    let tab_titles: Vec<Line> = ["  Order Book  ", "  Trade Book  ", "  Place Order  "]
        .iter()
        .map(|t| Line::from(*t))
        .collect();
    let selected = match app.tab {
        Tab::OrderBook => 0,
        Tab::TradeBook => 1,
        Tab::PlaceOrder => 2,
    };
    let tabs = Tabs::new(tab_titles)
        .select(selected)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Rust Exchange   [Tab] switch tab   [q] quit "),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::raw(" │ "));
    f.render_widget(tabs, layout[0]);

    match app.tab {
        Tab::OrderBook => render_order_book(f, app, layout[1]),
        Tab::TradeBook => render_trade_book(f, app, layout[1]),
        Tab::PlaceOrder => render_place_order(f, app, layout[1]),
    }

    let style = if app.status_err {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    let status = Paragraph::new(format!(" {}", app.status))
        .style(style)
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(status, layout[2]);
}

fn render_order_book(f: &mut Frame, app: &App, area: Rect) {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let pair_spans: Vec<Span> = PAIRS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == app.pair_idx {
                Span::styled(
                    format!("  ▶ {} ◀  ", p),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("    {}    ", p), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(pair_spans))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Pair   ◄/► or h/l to change   [r] refresh "),
            ),
        v[0],
    );

    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let header = Row::new([
        Cell::from("Price (USD)").style(header_style),
        Cell::from("Volume").style(header_style),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .order_book
        .iter()
        .map(|(price, vol)| {
            Row::new([
                Cell::from(format!("{:.2}", price)),
                Cell::from(format!("{:.8}", vol)),
            ])
        })
        .collect();

    f.render_widget(
        Table::new(rows, [Constraint::Percentage(50), Constraint::Percentage(50)])
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Order Book — {} ", PAIRS[app.pair_idx])),
            ),
        v[1],
    );
}

fn render_trade_book(f: &mut Frame, app: &App, area: Rect) {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let input_style = if app.trader_editing {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    let hint = if app.trader_editing {
        "  Enter to search   Esc to cancel"
    } else {
        "  [i] or [Enter] to edit"
    };
    f.render_widget(
        Paragraph::new(app.trader_input.as_str())
            .style(input_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Trader{} ", hint)),
            ),
        v[0],
    );

    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let header = Row::new([
        Cell::from("Status").style(header_style),
        Cell::from("ID").style(header_style),
        Cell::from("Pair").style(header_style),
        Cell::from("Side").style(header_style),
        Cell::from("Price").style(header_style),
        Cell::from("Volume").style(header_style),
        Cell::from("Timestamp").style(header_style),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .trades
        .iter()
        .map(|t| {
            let side_color = if t.side == "buy" {
                Color::Green
            } else {
                Color::Red
            };
            Row::new([
                Cell::from(t.status.as_str()),
                Cell::from(t.id.as_str()),
                Cell::from(t.pair.as_str()),
                Cell::from(Span::styled(
                    t.side.as_str(),
                    Style::default().fg(side_color),
                )),
                Cell::from(format!("{:.2}", t.price)),
                Cell::from(format!("{:.8}", t.volume)),
                Cell::from(t.timestamp.as_str()),
            ])
        })
        .collect();

    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(20),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Min(0),
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Trade Book ")),
        v[1],
    );
}

fn render_place_order(f: &mut Frame, app: &App, area: Rect) {
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let form = &app.form;
    let fv = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(h[0]);

    let focused = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let normal = Style::default().fg(Color::White);
    let bs = |field: usize| if form.field == field { focused } else { normal };

    // Pair
    let pair_spans: Vec<Span> = PAIRS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == form.pair_idx {
                Span::styled(
                    format!(" [{}] ", p),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {}  ", p), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(pair_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Pair  ◄/► ")
                .border_style(bs(0)),
        ),
        fv[0],
    );

    // Side
    let sides = [("buy", Color::Green), ("sell", Color::Red)];
    let side_spans: Vec<Span> = sides
        .iter()
        .enumerate()
        .map(|(i, (s, c))| {
            if i == form.side_idx {
                Span::styled(
                    format!(" [{}] ", s),
                    Style::default().fg(*c).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {}  ", s), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(side_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Side  ◄/► ")
                .border_style(bs(1)),
        ),
        fv[1],
    );

    // Order type
    let types = ["market", "limit"];
    let type_spans: Vec<Span> = types
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == form.type_idx {
                Span::styled(
                    format!(" [{}] ", t),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {}  ", t), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(type_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Order Type  ◄/► ")
                .border_style(bs(2)),
        ),
        fv[2],
    );

    // Price
    f.render_widget(
        Paragraph::new(form.price.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Price ")
                .border_style(bs(3)),
        ),
        fv[3],
    );

    // Volume
    f.render_widget(
        Paragraph::new(form.volume.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Volume ")
                .border_style(bs(4)),
        ),
        fv[4],
    );

    // Trader / submit
    let trader_title = if form.field == 5 {
        " Trader   [Enter] to submit "
    } else {
        " Trader "
    };
    f.render_widget(
        Paragraph::new(form.trader.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(trader_title)
                .border_style(bs(5)),
        ),
        fv[5],
    );

    // Hint
    f.render_widget(
        Paragraph::new(
            " ↑/↓  navigate   ◄/►  toggle options   Enter  advance / submit   Esc  reset",
        )
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center),
        fv[6],
    );

    // Order preview
    let preview = format!(
        "\n  Pair:        {}\n  Side:        {}\n  Order Type:  {}\n  Price:       {}\n  Volume:      {}\n  Trader:      {}",
        PAIRS[form.pair_idx],
        sides[form.side_idx].0,
        types[form.type_idx],
        if form.price.is_empty() { "—".to_string() } else { form.price.clone() },
        if form.volume.is_empty() { "—".to_string() } else { form.volume.clone() },
        if form.trader.is_empty() { "—".to_string() } else { form.trader.clone() },
    );
    f.render_widget(
        Paragraph::new(preview)
            .block(Block::default().borders(Borders::ALL).title(" Order Preview ")),
        h[1],
    );
}