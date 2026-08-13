//! The stock composition: the OSS product as shipped — every screen
//! from the library, no extensions. A distribution (e.g. a hosted
//! overlay's web crate) is this same two-liner with its extensions
//! passed in.

fn main() {
    converge_web::run(converge_web::Extensions::default());
}
