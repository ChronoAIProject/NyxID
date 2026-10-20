//! Small, independently deployable audit witness surface. The application server
//! integrates this client through its audit runtime; the sink never needs Mongo.
pub mod models {
    pub mod audit_witness;
}
pub mod services {
    pub mod audit_witness;
}
pub mod handlers {
    pub mod audit_witness;
}
