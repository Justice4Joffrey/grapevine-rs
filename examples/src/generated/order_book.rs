#[derive(Clone, PartialEq, ::prost::Message)]
pub struct QuoteDelta {
    #[prost(enumeration="QuoteAction", required, tag="1")]
    pub action: i32,
    #[prost(uint32, required, tag="2")]
    pub price: u32,
    #[prost(uint32, required, tag="3")]
    pub quantity: u32,
    #[prost(int64, required, tag="4")]
    pub timestamp: i64,
    #[prost(enumeration="Side", required, tag="5")]
    pub side: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum QuoteAction {
    Insert = 1,
    Remove = 2,
}
impl QuoteAction {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            QuoteAction::Insert => "INSERT",
            QuoteAction::Remove => "REMOVE",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Side {
    Buy = 1,
    Sell = 2,
}
impl Side {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }
}
