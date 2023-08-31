use std::{
    cmp::Reverse,
    collections::BTreeMap,
    convert::Infallible,
    fmt::{Display, Formatter},
};

use chrono::Utc;
use grapevine::StateSync;
use rand::prelude::*;
use rand_distr::Normal;

use crate::proto::order_book::*;

const MID_PRICE: f64 = 10000.0;
const STD_DEV: f64 = 2000.0;
const MIN_ORDERS: usize = 10;
const MAX_ORDERS: usize = 15;
const MAX_QUANTITY: u32 = 100;

/// An order book which just keeps track of the total amount of liquidity
/// available.
#[derive(Debug, Default)]
pub struct Level1OrderBook {
    buys: BTreeMap<Reverse<u32>, QuoteAggregate>,
    sells: BTreeMap<u32, QuoteAggregate>,
}

impl Display for Level1OrderBook {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let best_buy = self.buys.keys().min().map(|k| {
            let e = &self.buys[k];
            (e.quantity, e.price)
        });
        let best_sell = self.sells.keys().min().map(|k| {
            let e = &self.sells[k];
            (e.price, e.quantity)
        });

        write!(f, "Buy: {:?} | {:?} :Sell", best_buy, best_sell)
    }
}

/// An aggregate of [QuoteDelta]s.
#[derive(Debug, Default)]
pub struct QuoteAggregate {
    price: u32,
    quantity: u32,
    last_updated: i64,
}

impl Level1OrderBook {
    /// Create a new order book.
    pub fn new() -> Self {
        Self::default()
    }

    /// For demonstration purposes, generate a random stream of
    /// [QuoteDelta]s.
    pub fn new_random_delta(&self) -> QuoteDelta {
        let (side, n_orders, cross_limit) = if rand::random() {
            (
                Side::Buy,
                self.buys.len(),
                self.sells.keys().min().map(|x| x - 1),
            )
        } else {
            (
                Side::Sell,
                self.sells.len(),
                self.buys.keys().min().map(|x| x.0 + 1),
            )
        };

        // only start reducing orders once there are a few.
        let action = if n_orders < MIN_ORDERS {
            QuoteAction::Insert
        } else if n_orders < MAX_ORDERS {
            if rand::random() {
                QuoteAction::Remove
            } else {
                QuoteAction::Insert
            }
        } else {
            QuoteAction::Remove
        };

        let timestamp = Utc::now().timestamp_nanos();
        match action {
            QuoteAction::Insert => {
                // our fictional instrument trades somewhere around this price.
                let new_price = Normal::new(MID_PRICE, STD_DEV)
                    .unwrap()
                    .sample(&mut thread_rng()) as u32;
                let new_quantity = thread_rng().gen_range(1..MAX_QUANTITY);
                // don't cross the limit price.
                let price = match side {
                    Side::Buy => cross_limit.map(|l| l.min(new_price)).unwrap_or(new_price),
                    Side::Sell => cross_limit.map(|l| l.max(new_price)).unwrap_or(new_price),
                };
                QuoteDelta {
                    action: action as i32,
                    price,
                    quantity: new_quantity,
                    timestamp,
                    side: side as i32,
                }
            }

            QuoteAction::Remove => {
                let (price, quantity) = match side {
                    Side::Buy => {
                        let agg = self.buys.values().choose(&mut thread_rng()).unwrap();
                        let quantity = thread_rng().gen_range(1..=agg.quantity);
                        (agg.price, quantity)
                    }
                    Side::Sell => {
                        let agg = self.sells.values().choose(&mut thread_rng()).unwrap();
                        let quantity = thread_rng().gen_range(1..=agg.quantity);
                        (agg.price, quantity)
                    }
                };
                QuoteDelta {
                    action: action as i32,
                    price,
                    quantity,
                    timestamp,
                    side: side as i32,
                }
            }
        }
    }
}

impl QuoteAggregate {
    fn apply_delta(&mut self, delta: QuoteDelta) -> bool {
        match QuoteAction::from_i32(delta.action).unwrap() {
            QuoteAction::Insert => {
                self.quantity += delta.quantity;
                self.last_updated = delta.timestamp;
                true
            }
            QuoteAction::Remove => {
                self.quantity -= delta.quantity;
                self.last_updated = delta.timestamp;
                self.quantity > 0
            }
        }
    }
}

impl StateSync for Level1OrderBook {
    type ApplyError = Infallible;
    type Delta = QuoteDelta;

    fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError> {
        let price = delta.price;
        match Side::from_i32(delta.side).unwrap() {
            Side::Buy => {
                let retain = self
                    .buys
                    .entry(Reverse(price))
                    .or_insert(QuoteAggregate {
                        price,
                        ..Default::default()
                    })
                    .apply_delta(delta);
                if !retain {
                    self.buys.remove(&Reverse(price));
                }
            }
            Side::Sell => {
                let retain = self
                    .sells
                    .entry(price)
                    .or_insert(QuoteAggregate {
                        price,
                        ..Default::default()
                    })
                    .apply_delta(delta);
                if !retain {
                    self.sells.remove(&price);
                }
            }
        };
        Ok(())
    }
}
