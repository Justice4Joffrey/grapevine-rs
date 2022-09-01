/// / A self contained piece of state holding multiple dummy values.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Leaf {
    #[prost(int32, required, tag="1")]
    pub id: i32,
    #[prost(int32, required, tag="2")]
    pub a: i32,
    #[prost(int32, required, tag="3")]
    pub b: i32,
}
/// / Edit a leaf's value by id.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EditLeaf {
    #[prost(int32, required, tag="1")]
    pub id: i32,
    #[prost(int32, required, tag="2")]
    pub value: i32,
}
/// / Edit a leaf's 'a' value.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EditA {
    #[prost(message, required, tag="1")]
    pub leaf: EditLeaf,
}
/// / Edit a leaf's 'b' value.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EditB {
    #[prost(message, required, tag="1")]
    pub leaf: EditLeaf,
}
/// / Delete a leaf by id.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DeleteLeaf {
    #[prost(int32, required, tag="1")]
    pub id: i32,
}
/// / Create a new leaf.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct NewLeaf {
    #[prost(message, required, tag="1")]
    pub leaf: Leaf,
}
/// / All messages to change the state of the tree.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LeafDelta {
    #[prost(oneof="leaf_delta::Delta", tags="1, 2, 3, 4")]
    pub delta: ::core::option::Option<leaf_delta::Delta>,
}
/// Nested message and enum types in `LeafDelta`.
pub mod leaf_delta {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Delta {
        #[prost(message, tag="1")]
        EditA(super::EditA),
        #[prost(message, tag="2")]
        EditB(super::EditB),
        #[prost(message, tag="3")]
        DeleteLeaf(super::DeleteLeaf),
        #[prost(message, tag="4")]
        NewLeaf(super::NewLeaf),
    }
}
