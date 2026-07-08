//! converge-app — the real product, composed from converge-ui. Screens are
//! driven by a hash router; the sidebar, breadcrumb, and content all react to
//! the current route and the active group.

mod dashboard;
mod data;
mod decision_detail;
mod expert;
mod project_log;
mod route;
mod search;
mod seed;
mod settings;
mod signals;
mod source_viewer;
mod store;
mod when;

use converge_ui::atoms::{Avatar, Button, Glyph, Input};
use converge_ui::domain::{GroupKind, Tone};
use converge_ui::layout::AppShell;
use converge_ui::molecules::{NavItem, ProjectNavItem};
use dashboard::Dashboard;
use decision_detail::DecisionDetail;
use expert::Expert;
use leptos::ev;
use leptos::html;
use leptos::mount::mount_to_body;
use leptos::prelude::*;
use project_log::ProjectLog;
use route::{Route, current_route, navigate};
use search::Search;
use settings::Settings;
use signals::{SignalDetail, Signals};
use source_viewer::SourceViewer;
use store::{AppStateStoreFields, AppStore, LoadError, provide_default_store};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Every focusable element inside `container`, in DOM (i.e. tab) order.
#[cfg(target_arch = "wasm32")]
fn focusable_in(container: &web_sys::HtmlElement) -> Vec<web_sys::HtmlElement> {
    let Ok(list) = container
        .query_selector_all("a[href], button:not([disabled]), [tabindex]:not([tabindex='-1'])")
    else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.get(i))
        .filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
        .collect()
}

/// Whether `el` currently holds keyboard focus.
#[cfg(target_arch = "wasm32")]
fn is_focused(el: &web_sys::HtmlElement) -> bool {
    document()
        .active_element()
        .is_some_and(|active| active.is_same_node(el.dyn_ref::<web_sys::Node>()))
}

/// Install the drawer's keyboard contract and focus management. Browser-only:
/// it walks the DOM and moves focus by hand, so it's compiled out on non-wasm
/// targets (native test/`cargo test` builds), where there is no DOM.
#[cfg(target_arch = "wasm32")]
fn install_drawer_focus_management(
    nav_open: ReadSignal<bool>,
    set_nav_open: WriteSignal<bool>,
    sidebar_ref: NodeRef<html::Aside>,
    navbtn_ref: NodeRef<html::Button>,
) {
    // Drawer keyboard contract (WAI-ARIA dialog): Escape dismisses, and
    // Tab/Shift+Tab are trapped inside it while open so focus can't wander into
    // the content the scrim is hiding.
    window_event_listener(ev::keydown, move |evt| {
        if !nav_open.get_untracked() {
            return;
        }
        match evt.key().as_str() {
            "Escape" => set_nav_open.set(false),
            "Tab" => {
                let Some(aside) = sidebar_ref.get() else {
                    return;
                };
                let items = focusable_in(&aside);
                let (Some(first), Some(last)) = (items.first(), items.last()) else {
                    return;
                };
                if evt.shift_key() {
                    if is_focused(first) {
                        evt.prevent_default();
                        let _ = last.focus();
                    }
                } else if is_focused(last) {
                    evt.prevent_default();
                    let _ = first.focus();
                }
            }
            _ => {}
        }
    });

    // Move focus into the drawer on open and back to the toggle on close, so
    // keyboard users always land somewhere sensible.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = nav_open.get();
        if is_open && was_open != Some(true) {
            if let Some(aside) = sidebar_ref.get() {
                match focusable_in(&aside).into_iter().next() {
                    Some(el) => {
                        let _ = el.focus();
                    }
                    None => {
                        let _ = aside.focus();
                    }
                }
            }
        } else if !is_open
            && was_open == Some(true)
            && let Some(btn) = navbtn_ref.get()
        {
            let _ = btn.focus();
        }
        is_open
    });
}

/// No-op stand-in for non-wasm builds (see the wasm version above).
#[cfg(not(target_arch = "wasm32"))]
fn install_drawer_focus_management(
    _nav_open: ReadSignal<bool>,
    _set_nav_open: WriteSignal<bool>,
    _sidebar_ref: NodeRef<html::Aside>,
    _navbtn_ref: NodeRef<html::Button>,
) {
}

fn main() {
    apply_theme(&read_theme());
    mount_to_body(|| view! { <App /> });
}

fn read_theme() -> String {
    document()
        .document_element()
        .and_then(|el| el.get_attribute("data-theme"))
        .unwrap_or_else(|| "dark".into())
}

fn apply_theme(t: &str) {
    if let Some(el) = document().document_element() {
        let _ = el.set_attribute("data-theme", t);
    }
}

#[component]
fn App() -> impl IntoView {
    // The store owns the whole dataset + active-group state, and is published to
    // context for every descendant. `store` is a copyable handle we also keep
    // here for the router and the group switcher. The dataset loads
    // asynchronously; until it resolves the UI shows a loading screen.
    let store = provide_default_store();
    let (route, set_route) = signal(current_route());
    // Drawer state for narrow viewports: the sidebar is off-canvas there,
    // opened by the top-bar hamburger. Harmless on desktop, where the CSS keeps
    // the sidebar in the grid.
    let (nav_open, set_nav_open) = signal(false);
    let sidebar_ref = NodeRef::<html::Aside>::new();
    let navbtn_ref = NodeRef::<html::Button>::new();

    // One entry point for navigation: update the URL hash and the signal. Any
    // navigation also closes the drawer.
    let go = Callback::new(move |r: Route| {
        set_nav_open.set(false);
        navigate(&r);
        set_route.set(r);
    });
    // Switching the active group resets to the dashboard, like the prototype.
    let switch_group = Callback::new(move |i: usize| {
        store.group().set(i);
        set_nav_open.set(false);
        navigate(&Route::Dashboard);
        set_route.set(Route::Dashboard);
    });
    // Browser back/forward re-syncs the signal from the hash — and closes the
    // drawer, so a route change from any source (link, back/forward, URL) leaves
    // it shut.
    window_event_listener(ev::hashchange, move |_| {
        set_nav_open.set(false);
        set_route.set(current_route());
    });
    // "/" jumps to Search (unless already typing in a field).
    window_event_listener(ev::keydown, move |evt| {
        if evt.key() == "/" && !evt.meta_key() && !evt.ctrl_key() && !evt.alt_key() {
            if let Some(el) = document().active_element() {
                let tag = el.tag_name();
                if tag == "INPUT" || tag == "TEXTAREA" {
                    return;
                }
            }
            evt.prevent_default();
            go.run(Route::Search);
        }
    });
    install_drawer_focus_management(nav_open, set_nav_open, sidebar_ref, navbtn_ref);

    // Crossing back over the drawer breakpoint (e.g. rotating a tablet) closes
    // it, so nav_open can't linger into desktop layout where the trap would
    // keep grabbing Tab for a menu that's part of the grid again.
    window_event_listener(ev::resize, move |_| {
        if nav_open.get_untracked() {
            let wide = window()
                .inner_width()
                .ok()
                .and_then(|w| w.as_f64())
                .is_some_and(|w| w >= 1024.0);
            if wide {
                set_nav_open.set(false);
            }
        }
    });

    // Background scroll-lock while the drawer is open — reflected onto <html>
    // so the CSS can freeze `.cv-shell__scroll` (which lives inside AppShell).
    Effect::new(move |_| {
        if let Some(el) = document().document_element() {
            let _ = el.set_attribute(
                "data-nav-open",
                if nav_open.get() { "true" } else { "false" },
            );
        }
    });

    // Gate the whole shell on the load: the sidebar and every screen query the
    // dataset, so none of it can render until the store is populated.
    move || {
        if let Some(err) = store.error().get() {
            return match err {
                LoadError::Unauthorized => login().into_any(),
                LoadError::Failed(msg) => boot_error(msg).into_any(),
            };
        }
        let Some(dataset) = store.dataset().get() else {
            return boot_loading().into_any();
        };
        // A fresh deployment has no groups yet; every accessor below
        // assumes at least one, so gate with an honest empty state.
        if dataset.groups.is_empty() {
            return boot_empty().into_any();
        }
        view! {
            <AppShell
                sidebar=view! {
                    <Sidebar
                        route=route
                        store=store
                        go=go
                        switch_group=switch_group
                        nav_open=nav_open
                        set_nav_open=set_nav_open
                        sidebar_ref=sidebar_ref
                    />
                }
                    .into_any()
                topbar=view! {
                    <TopBar
                        route=route
                        store=store
                        go=go
                        nav_open=nav_open
                        set_nav_open=set_nav_open
                        navbtn_ref=navbtn_ref
                    />
                }
                    .into_any()
            >
                {move || {
                    let _ = store.group().get();
                    match route.get() {
                        Route::Dashboard => view! { <Dashboard go=go /> }.into_any(),
                        Route::Decision(id) => view! { <DecisionDetail go=go id=id /> }.into_any(),
                        Route::Signals => view! { <Signals go=go /> }.into_any(),
                        Route::SignalDetail(id) => view! { <SignalDetail go=go id=id /> }.into_any(),
                        Route::Source(id, idx) => view! { <SourceViewer go=go id=id idx=idx /> }.into_any(),
                        Route::Project(id) => view! { <ProjectLog go=go pid=id /> }.into_any(),
                        Route::Search => view! { <Search go=go /> }.into_any(),
                        Route::Expert => view! { <Expert /> }.into_any(),
                        Route::Settings => view! { <Settings /> }.into_any(),
                    }
                }}
            </AppShell>
        }
        .into_any()
    }
}

/// Full-screen loading state shown while the dataset resolves.
fn boot_loading() -> impl IntoView {
    view! {
        <div class="cv-boot cv-fg-muted cv-fs-lg">
            <div class="cv-row cv-gap-10">
                <span class="cv-livedot"></span>
                "Loading decision memory…"
            </div>
        </div>
    }
}

/// An interrupted flow to resume after sign-in (`?next=`, e.g. an MCP
/// connector's authorize URL) — same-origin paths only. Browser-only:
/// it reads the query string through `web-sys`. Only the `api` build's
/// submit path calls it (the embedded fixture never signs in).
#[cfg_attr(not(feature = "api"), allow(dead_code))]
#[cfg(target_arch = "wasm32")]
fn resume() -> Option<String> {
    window()
        .location()
        .search()
        .ok()
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|q| q.get("next"))
        .filter(|n| n.starts_with('/') && !n.starts_with("//"))
}

/// No-op stand-in for non-wasm builds (see the wasm version above).
#[cfg_attr(not(feature = "api"), allow(dead_code))]
#[cfg(not(target_arch = "wasm32"))]
fn resume() -> Option<String> {
    None
}

/// Full-screen login: exchange a pasted bearer token for the session
/// cookie, then reload — the boot load then succeeds with the cookie
/// riding fetch ambiently. The pasted secret is sent once and never
/// stored by the app.
fn login() -> impl IntoView + use<> {
    let (token, set_token) = signal(String::new());
    let (notice, set_notice) = signal(None::<String>);
    // Does this deployment offer IdP sign-in? (Open capability read; the
    // embedded build has no API to ask.)
    let (idp, set_idp) = signal(None::<String>);
    #[cfg(feature = "api")]
    leptos::task::spawn_local(async move {
        if let Ok(info) = crate::store::client().auth_info().await {
            set_idp.set(info.oidc);
        }
    });
    #[cfg(not(feature = "api"))]
    let _ = set_idp;
    #[cfg(feature = "api")]
    let submit = move || {
        let secret = token.get_untracked();
        if secret.trim().is_empty() {
            return;
        }
        leptos::task::spawn_local(async move {
            use converge_client::StoreError;
            match crate::store::client().session_login(secret.trim()).await {
                Ok(()) => {
                    let _ = match resume() {
                        Some(next) => window().location().assign(&next),
                        None => window().location().reload(),
                    };
                }
                Err(StoreError::Unauthorized) => {
                    set_notice.set(Some("That token isn't recognized.".into()));
                }
                Err(e) => set_notice.set(Some(format!("Sign-in failed: {e}"))),
            }
        });
    };
    // The embedded fixture never asks for login; the arm exists so the
    // screen compiles (and stays previewable) in both builds.
    #[cfg(not(feature = "api"))]
    let submit = move || {
        let _ = token.get_untracked();
        set_notice.set(Some("This build has no API to sign in to.".into()));
    };
    view! {
        <div class="cv-boot">
            <div class="cv-boot__msg cv-text-center">
                <div class="cv-fs-xl cv-fw-semibold cv-mb-8">"Sign in to Converge"</div>
                <div class="cv-fs-md cv-fg-muted cv-lh-normal cv-mb-16">
                    "Paste a bearer token — your operator mints one with "
                    <span class="cv-mono">"converge-server token mint"</span> "."
                </div>
                <div class="cv-input cv-mb-8">
                    <input
                        class="cv-input__field"
                        type="password"
                        placeholder="cvg_…"
                        prop:value=token
                        on:input=move |ev| set_token.set(event_target_value(&ev))
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" {
                                submit();
                            }
                        }
                    />
                </div>
                <Button label="Sign in" on_click=Callback::new(move |()| submit()) />
                {move || {
                    idp.get()
                        .map(|label| {
                            view! {
                                <div class="cv-mt-16">
                                    <a class="cv-btn cv-btn--outline cv-btn--neutral" href="/auth/login">
                                        {format!("Sign in with {label}")}
                                    </a>
                                </div>
                            }
                        })
                }}
                {move || {
                    notice
                        .get()
                        .map(|msg| {
                            view! {
                                <div class="cv-fs-sm cv-fg-danger cv-mt-8">{msg}</div>
                            }
                        })
                }}
            </div>
        </div>
    }
}

/// Fresh deployment, nothing recorded yet. Creating groups lives on the
/// API/MCP surfaces for now — say so instead of rendering a shell that
/// assumes data.
fn boot_empty() -> impl IntoView {
    view! {
        <div class="cv-boot">
            <div class="cv-boot__msg cv-text-center">
                <div class="cv-fs-xl cv-fw-semibold cv-mb-8">"Nothing here yet"</div>
                <div class="cv-fs-md cv-fg-muted cv-lh-normal">
                    "Decision memory is empty. Create a group and project over the API, "
                    "or connect an agent to " <span class="cv-mono">"/mcp"</span>
                    " and record the first decision."
                </div>
            </div>
        </div>
    }
}

/// Full-screen error state shown when the load fails.
fn boot_error(msg: String) -> impl IntoView {
    view! {
        <div class="cv-boot">
            <div class="cv-boot__msg cv-text-center">
                <div class="cv-fs-xl cv-fw-semibold cv-fg-danger cv-mb-8">
                    "Couldn't load decision memory"
                </div>
                <div class="cv-fs-md cv-fg-muted cv-lh-normal">{msg}</div>
            </div>
        </div>
    }
}

#[component]
fn Sidebar(
    route: ReadSignal<Route>,
    store: AppStore,
    go: Callback<Route>,
    switch_group: Callback<usize>,
    nav_open: ReadSignal<bool>,
    set_nav_open: WriteSignal<bool>,
    sidebar_ref: NodeRef<html::Aside>,
) -> impl IntoView {
    let acct = data::account();
    let (group_open, set_group_open) = signal(false);
    let (acct_open, set_acct_open) = signal(false);
    let (theme, set_theme) = signal(read_theme());

    let toggle_theme = move |_| {
        let next = if theme.get_untracked() == "light" {
            "dark"
        } else {
            "light"
        };
        apply_theme(next);
        set_theme.set(next.to_string());
    };

    // Clear the session cookie, then reload into the login screen. Against
    // the embedded fixture there is no session — just close the menu.
    let logout = move |_| {
        set_acct_open.set(false);
        #[cfg(feature = "api")]
        leptos::task::spawn_local(async {
            let _ = crate::store::client().session_logout().await;
            let _ = window().location().reload();
        });
    };

    view! {
        // Scrim behind the off-canvas drawer (narrow viewports only; the CSS
        // hides it on desktop). Clicking it closes the drawer.
        {move || {
            nav_open.get().then(|| view! {
                <div class="cv-navscrim" on:click=move |_| set_nav_open.set(false)></div>
            })
        }}
        <aside
            node_ref=sidebar_ref
            class=move || {
                if nav_open.get() { "cv-sidebar cv-sidebar--open" } else { "cv-sidebar" }
            }
            role=move || if nav_open.get() { "dialog" } else { "navigation" }
            aria-modal=move || if nav_open.get() { "true" } else { "false" }
            aria-label="Main navigation"
        >
            <div class="cv-sidebar__logo">
                <div class="cv-sidebar__brand"></div>
                <div class="cv-col cv-lh-1">
                    <span class="cv-fw-semibold cv-fs-xl">"Converge"</span>
                    <span class="cv-fs-xs cv-fg-faint">"decision memory"</span>
                </div>
            </div>

            // group selector + switcher dropdown
            <div class="cv-relative">
                <div
                    class="cv-sidebar__group"
                    role="button"
                    tabindex="0"
                    on:click=move |_| set_group_open.update(|o| *o = !*o)
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" || ev.key() == " " {
                            ev.prevent_default();
                            set_group_open.update(|o| *o = !*o);
                        }
                    }
                >
                    <div class="cv-groupicon">
                        {move || {
                            store.group().get();
                            match data::cur_group().kind {
                                GroupKind::Personal => Glyph::Personal.glyph(),
                                GroupKind::Shared => Glyph::Shared.glyph(),
                            }
                        }}
                    </div>
                    <div class="cv-grow">
                        <div class="cv-fw-medium cv-fs-md">
                            {move || { store.group().get(); data::group_name() }}
                        </div>
                        <div class="cv-fs-2xs cv-fg-faint">
                            {move || { store.group().get(); data::group_meta() }}
                        </div>
                    </div>
                    <span class="cv-fg-muted">{Glyph::CaretDown.glyph()}</span>
                </div>
                {move || {
                    store.group().get();
                    group_open.get().then(|| {
                        let (shared, personal): (Vec<_>, Vec<_>) = data::groups()
                            .into_iter()
                            .enumerate()
                            .partition(|(_, g)| g.kind == GroupKind::Shared);
                        view! {
                            <div class="cv-groupmenu">
                                <div class="cv-groupmenu__label">"Shared groups"</div>
                                {shared.into_iter().map(|(i, g)| group_row(i, &g, store, switch_group, set_group_open)).collect_view()}
                                <div class="cv-groupmenu__label cv-groupmenu__label--div">"Personal"</div>
                                {personal.into_iter().map(|(i, g)| group_row(i, &g, store, switch_group, set_group_open)).collect_view()}
                            </div>
                        }
                    })
                }}
            </div>

            <div>
                <div class="cv-sidebar__section">"Views"</div>
                <nav class="cv-col cv-gap-2">
                    {move || {
                        let r = route.get();
                        let _ = store.group().get();
                        let sig_count = data::group_signals().len() as u32;
                        view! {
                            <NavItem
                                icon=Glyph::Dashboard
                                label="Dashboard"
                                active=r == Route::Dashboard
                                on_click=Callback::new(move |_| go.run(Route::Dashboard))
                            />
                            <NavItem
                                icon=Glyph::Search
                                label="Search"
                                active=r == Route::Search
                                on_click=Callback::new(move |_| go.run(Route::Search))
                            />
                            <NavItem
                                icon=Glyph::Signal
                                label="Signals"
                                count=sig_count
                                accent=Tone::Signal
                                active=r == Route::Signals
                                on_click=Callback::new(move |_| go.run(Route::Signals))
                            />
                            <NavItem
                                icon=Glyph::Expert
                                label="Expert model"
                                accent=Tone::Expert
                                active=r == Route::Expert
                                on_click=Callback::new(move |_| go.run(Route::Expert))
                            />
                        }
                    }}
                </nav>
            </div>

            <div class="cv-col cv-fill">
                <div class="cv-sidebar__section">"Projects"</div>
                <div class="cv-sidebar__projects">
                    {move || {
                        store.group().get();
                        data::cur_group_projects()
                            .iter()
                            .map(move |p| {
                                let name = data::proj_name(p);
                                let pn = p.to_string();
                                let unread = data::unread_count(p);
                                view! {
                                    <ProjectNavItem
                                        name=name
                                        unread=unread
                                        on_click=Callback::new(move |_| go.run(Route::Project(pn.clone())))
                                    />
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </div>

            // account + menu
            <div class="cv-relative">
                {move || {
                    acct_open.get().then(move || {
                        let acct = data::account();
                        view! {
                            <div class="cv-acctmenu__scrim" on:click=move |_| set_acct_open.set(false)></div>
                            <div class="cv-acctmenu">
                                <div class="cv-acctmenu__head">
                                    <Avatar initial=acct.initial.clone() color=acct.color.clone() size=34 />
                                    <div class="cv-minw-0">
                                        <div class="cv-acctmenu__name">{acct.name.clone()}</div>
                                        <div class="cv-acctmenu__email cv-mono" title=acct.email.clone()>{acct.email.clone()}</div>
                                    </div>
                                </div>
                                <div class="cv-acctmenu__sep"></div>
                                <div
                                    class="cv-acctmenu__item"
                                    on:click=move |_| {
                                        set_acct_open.set(false);
                                        go.run(Route::Settings);
                                    }
                                >
                                    <span class="cv-iconcell cv-fg-muted">
                                        {Glyph::Settings.glyph()}
                                    </span>
                                    " Settings"
                                </div>
                                <div class="cv-acctmenu__item" on:click=toggle_theme>
                                    <span class="cv-iconcell cv-fg-muted">
                                        {Glyph::Personal.glyph()}
                                    </span>
                                    " Theme"
                                    <span class="cv-spacer"></span>
                                    <span class="cv-mono cv-fs-xs cv-fg-faint">
                                        {move || if theme.get() == "light" { "Light" } else { "Dark" }}
                                    </span>
                                </div>
                                <div class="cv-acctmenu__sep"></div>
                                <div class="cv-acctmenu__item cv-acctmenu__item--danger" on:click=logout>
                                    <span class="cv-iconcell">{Glyph::Power.glyph()}</span>
                                    " Log out"
                                </div>
                            </div>
                        }
                    })
                }}
                <div
                    class="cv-sidebar__account"
                    role="button"
                    tabindex="0"
                    on:click=move |_| set_acct_open.update(|o| *o = !*o)
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" || ev.key() == " " {
                            ev.prevent_default();
                            set_acct_open.update(|o| *o = !*o);
                        }
                    }
                >
                    <Avatar initial=acct.initial.clone() color=acct.color.clone() size=28 />
                    <div class="cv-grow">
                        <div class="cv-fs-md cv-fw-medium">{acct.name.clone()}</div>
                        <div class="cv-fs-2xs cv-fg-faint">{acct.role.clone()}</div>
                    </div>
                    <span class="cv-fg-faint">{Glyph::More.glyph()}</span>
                </div>
            </div>
        </aside>
    }
}

/// One row in the group-switcher dropdown.
fn group_row(
    i: usize,
    g: &data::GroupDef,
    store: AppStore,
    switch_group: Callback<usize>,
    set_group_open: WriteSignal<bool>,
    // Everything the view holds is owned; opt out of edition 2024's
    // capture-all-lifetimes default so `g`'s borrow ends at the call.
) -> impl IntoView + use<> {
    let icon = match g.kind {
        GroupKind::Personal => Glyph::Personal.glyph(),
        GroupKind::Shared => Glyph::Shared.glyph(),
    };
    let count = format!(
        "{} {}",
        g.project_ids.len(),
        if g.project_ids.len() == 1 {
            "project"
        } else {
            "projects"
        }
    );
    let name = g.name.clone();
    view! {
        <div
            class="cv-groupmenu__row"
            on:click=move |_| {
                switch_group.run(i);
                set_group_open.set(false);
            }
        >
            <span class="cv-groupmenu__icon">{icon}</span>
            <span class="cv-grow cv-truncate">
                {name}
            </span>
            <span class="cv-fs-2xs cv-fg-faint">{count}</span>
            {move || (store.group().get() == i).then(|| view! { <span class="cv-fg-primary cv-fs-xs">{Glyph::Verified.glyph()}</span> })}
        </div>
    }
}

#[component]
fn TopBar(
    route: ReadSignal<Route>,
    store: AppStore,
    go: Callback<Route>,
    nav_open: ReadSignal<bool>,
    set_nav_open: WriteSignal<bool>,
    navbtn_ref: NodeRef<html::Button>,
) -> impl IntoView {
    view! {
        // Hamburger — visible only on narrow viewports (CSS), toggles the drawer.
        // Three real bars (not a glyph) so they stay pixel-crisp and can morph
        // into a close "X" via CSS alone.
        <button
            node_ref=navbtn_ref
            class=move || {
                if nav_open.get() { "cv-navbtn cv-navbtn--open" } else { "cv-navbtn" }
            }
            aria-label="Open navigation"
            aria-expanded=move || nav_open.get().to_string()
            on:click=move |_| set_nav_open.update(|o| *o = !*o)
        >
            <span class="cv-navbtn__bar"></span>
            <span class="cv-navbtn__bar"></span>
            <span class="cv-navbtn__bar"></span>
        </button>
        <div class="cv-topbar__crumb">
            <span class="cv-pointer" on:click=move |_| go.run(Route::Dashboard)>
                {move || { store.group().get(); data::group_name() }}
            </span>
            <span class="cv-topbar__sep">"/"</span>
            <span class="cv-topbar__cur">{move || route.get().crumb()}</span>
        </div>
        <div class="cv-topbar__spacer"></div>
        <div class="cv-topbar__search" on:click=move |_| go.run(Route::Search)>
            <Input placeholder="Search decisions across the group…" lead=Glyph::Search trail="/" />
        </div>
    }
}
