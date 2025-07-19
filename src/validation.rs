use crate::error::{ExchangeError, Result};

pub fn validate_trading_pair(pair: &str) -> Result<()> {
    if pair.is_empty() {
        return Err(ExchangeError::ValidationError("Trading pair cannot be empty".to_string()));
    }
    
    if pair.len() < 6 || pair.len() > 12 {
        return Err(ExchangeError::ValidationError("Trading pair must be between 6-12 characters".to_string()));
    }
    
    if !pair.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ExchangeError::ValidationError("Trading pair must contain only uppercase letters".to_string()));
    }
    
    Ok(())
}

pub fn validate_volume(volume: f64) -> Result<()> {
    if volume <= 0.0 {
        return Err(ExchangeError::ValidationError("Volume must be positive".to_string()));
    }
    
    if volume.is_infinite() || volume.is_nan() {
        return Err(ExchangeError::ValidationError("Volume must be a valid finite number".to_string()));
    }
    
    if volume > 1_000_000.0 {
        return Err(ExchangeError::ValidationError("Volume exceeds maximum allowed limit".to_string()));
    }
    
    Ok(())
}

pub fn validate_price(price: f64) -> Result<()> {
    if price < 0.0 {
        return Err(ExchangeError::ValidationError("Price cannot be negative".to_string()));
    }
    
    if price.is_infinite() || price.is_nan() {
        return Err(ExchangeError::ValidationError("Price must be a valid finite number".to_string()));
    }
    
    Ok(())
}

pub fn validate_trader_id(trader: &str) -> Result<()> {
    if trader.is_empty() {
        return Err(ExchangeError::ValidationError("Trader ID cannot be empty".to_string()));
    }
    
    if trader.len() > 50 {
        return Err(ExchangeError::ValidationError("Trader ID cannot exceed 50 characters".to_string()));
    }
    
    if !trader.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(ExchangeError::ValidationError("Trader ID can only contain alphanumeric characters, underscores, and hyphens".to_string()));
    }
    
    Ok(())
}

pub fn validate_order_side(side: &str) -> Result<()> {
    match side.to_lowercase().as_str() {
        "buy" | "sell" => Ok(()),
        _ => Err(ExchangeError::ValidationError("Order side must be 'buy' or 'sell'".to_string())),
    }
}

pub fn validate_order_type(order_type: &str) -> Result<()> {
    match order_type.to_lowercase().as_str() {
        "market" | "limit" => Ok(()),
        _ => Err(ExchangeError::ValidationError("Order type must be 'market' or 'limit'".to_string())),
    }
}

pub fn validate_order_request(request: &crate::models::model::models::orderbook::OrderRequest) -> Result<()> {
    validate_trading_pair(&request.pair)?;
    validate_volume(request.volume)?;
    validate_trader_id(&request.trader)?;
    validate_order_side(&request.side)?;
    validate_order_type(&request.order_type)?;
    
    if request.order_type.to_lowercase() == "limit" {
        validate_price(request.price)?;
        if request.price == 0.0 {
            return Err(ExchangeError::ValidationError("Limit orders must have a non-zero price".to_string()));
        }
    }
    
    Ok(())
}
