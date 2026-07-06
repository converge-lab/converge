//! The reactive application store.
//!
//! Everything the app renders lives in one `Store<AppState>` (from
//! `reactive_stores`), handed to every component through Leptos context. The
//! store gives *fine-grained* reactivity: updating `group` never notifies the
//! readers of `dataset`, and vice-versa — so a group switch doesn't invalidate
//! the whole tree.
//!
//! Data enters the store through a [`DataSource`]: [`EmbeddedSource`] parses
//! and assembles the embedded fixture seed (`crate::seed`); the HTTP
//! `ApiSource` on `converge-client` returns in the next slice. Both funnel
//! through `data::build_dataset`, so the two paths cannot drift.

use crate::data::{Dataset, build_dataset};
use crate::seed::{EMBEDDED, Seed, assemble};
use leptos::prelude::*;
use leptos::task::spawn_local;
use reactive_stores::Store;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// The whole reactive state of the app.
#[derive(Clone, Default, Store)]
pub struct AppState {
    /// The loaded dataset. `None` while the source is still resolving. The UI
    /// gates on this: nothing that queries the dataset renders until it's
    /// `Some`, so query functions can assume a loaded dataset.
    pub dataset: Option<Rc<Dataset>>,
    /// Set if loading failed. When present, the app shows an error screen.
    pub error: Option<String>,
    /// Index of the active group within `dataset.groups`.
    pub group: usize,
}

/// `Rc` inside the state makes it `!Send`, so the store is pinned to the single
/// WASM thread via `LocalStorage` — exactly right for a client-side app, and it
/// spares us needless `Send + Sync` bounds.
pub type AppStore = Store<AppState, LocalStorage>;

/// Why a load failed. Carries a human-readable message; the embedded source
/// never produces one, but an HTTP `ApiSource` will.
#[derive(Debug, Clone)]
pub struct LoadError(pub String);

/// A pending dataset load. The future is intentionally *not* `Send`: it runs on
/// the single WASM thread (via `spawn_local`) and an HTTP source's `fetch`
/// future is `!Send` anyway.
pub type Loading = Pin<Box<dyn Future<Output = Result<Rc<Dataset>, LoadError>>>>;

/// Where the dataset comes from. Async so an HTTP `ApiSource` slots in behind
/// the same trait without touching the store or any component.
pub trait DataSource {
    fn load(&self) -> Loading;
}

/// The embedded fixture seed, assembled in-process. Resolves immediately;
/// stays useful as the offline / test fixture even after a real API exists.
pub struct EmbeddedSource;

impl DataSource for EmbeddedSource {
    fn load(&self) -> Loading {
        Box::pin(async {
            let seed = Seed::parse(EMBEDDED).expect("embedded seed is malformed");
            // A broken seed is a build-time mistake; validation is a cheap
            // safety net in dev builds and dead weight in release.
            #[cfg(debug_assertions)]
            if let Err(errs) = crate::seed::validate(&seed) {
                panic!("embedded seed invalid:\n{}", errs.join("\n"));
            }
            Ok(Rc::new(build_dataset(assemble(&seed))))
        })
    }
}

/// Build an empty store, publish it to context, and kick off the async load
/// from `source`. Returns the (copyable) handle for the caller to keep. When
/// the load resolves it writes into the store, flipping the UI from its loading
/// state to the app (or to an error screen on failure).
pub fn provide_store<S: DataSource + 'static>(source: S) -> AppStore {
    let store = AppStore::new_local(AppState::default());
    provide_context(store);
    spawn_local(async move {
        match source.load().await {
            Ok(dataset) => store.dataset().set(Some(dataset)),
            Err(err) => store.error().set(Some(err.0)),
        }
    });
    store
}

/// Provide the store from the default source for this build. Today that is
/// the [`EmbeddedSource`]; the HTTP `ApiSource` (on `converge-client`)
/// slots back in here when it lands.
pub fn provide_default_store() -> AppStore {
    provide_store(EmbeddedSource)
}

/// The store for the current reactive owner.
///
/// Panics if [`provide_store`] wasn't called at the root — a programming error,
/// not a runtime condition.
pub fn use_store() -> AppStore {
    expect_context::<AppStore>()
}
