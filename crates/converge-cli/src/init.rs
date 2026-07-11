//! `converge project init` — bind the repository, from the terminal.
//!
//! This is the **manual fallback and reconfiguration** path; the main
//! path binds in-session through the agent (suggest → conversation →
//! hooks write the marker). The rule is the same on both (POC-validated):
//! a committed marker decides; a human decides when there is none; never
//! bind silently. Bound and disabled repos report and exit unless
//! `--rebind`; `--off` opts the repository out and needs no server.

use std::io::{BufRead, Write as _};
use std::path::Path;

use anyhow::{Context, Result, bail};
use converge_client::{Client, NewProject, Pagination, Project, ProjectFilter};

use crate::config::Config;
use crate::marker::{self, State};

pub async fn run(rebind: bool, off: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("current directory")?;
    let root = marker::root(&cwd);

    // Saying no must work offline: no config, no server round trip.
    if off {
        let path = marker::write_disabled(&root)?;
        println!(
            "converge is off for this repository ({}); commit it to make \
             that the team's answer",
            path.display()
        );
        return Ok(());
    }

    let state = marker::find(&cwd)?;
    let config = Config::load()?;
    let client = config.client()?;
    let me = client
        .me()
        .await
        .with_context(|| format!("cannot reach {} (check server/token)", config.server))?;
    println!("connected to {} as @{}", config.server, me.handle);

    match state {
        State::Bound { path, project } if !rebind => {
            let name = match client.project_get(project).await? {
                Some(project) => project.name,
                None => bail!(
                    "{} binds project {project}, which the server doesn't know — \
                     wrong server, or the marker predates a reset; \
                     `--rebind` to fix the binding",
                    path.display()
                ),
            };
            println!("already bound: \"{name}\" ({})", path.display());
            return Ok(());
        }
        State::Disabled { path } if !rebind => {
            println!(
                "converge is disabled here ({}) — `--rebind` to bind anyway",
                path.display()
            );
            return Ok(());
        }
        _ => {}
    }

    let projects = client
        .project_list(&ProjectFilter::default(), &Pagination::default())
        .await?
        .items;
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();

    println!("\nbind this repository:");
    for (i, project) in projects.iter().enumerate() {
        println!("  {}) {}", i + 1, describe(project));
    }
    println!("  n) create a new project");
    let choice = ask(&mut lines, "bind to")?;

    let (id, name) = if choice.eq_ignore_ascii_case("n") {
        create(&client, &mut lines, &root).await?
    } else {
        let index: usize = choice
            .parse()
            .ok()
            .filter(|i| (1..=projects.len()).contains(i))
            .with_context(|| format!("`{choice}` is not a listed option"))?;
        let project = &projects[index - 1];
        (project.id, project.name.clone())
    };

    let path = marker::write_bound(&root, id, &name)?;
    println!(
        "bound to \"{name}\" — wrote {}; commit it so the whole team resolves here",
        path.display()
    );
    Ok(())
}

/// Create-flow: name (defaulting to the directory name), group by prompt
/// only when there's a real choice.
async fn create(
    client: &Client,
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    root: &Path,
) -> Result<(converge_client::ProjectId, String)> {
    let default = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = match ask(lines, &format!("project name [{default}]"))? {
        s if s.is_empty() => default,
        s => s,
    };

    let groups = client.group_list(&Pagination::default()).await?.items;
    let group = match groups.len() {
        0 => bail!("the server has no groups (it always bootstraps one — check the deployment)"),
        1 => &groups[0],
        _ => {
            println!("which group owns it?");
            for (i, group) in groups.iter().enumerate() {
                println!("  {}) {}", i + 1, group.name);
            }
            let choice = ask(lines, "group")?;
            let index: usize = choice
                .parse()
                .ok()
                .filter(|i| (1..=groups.len()).contains(i))
                .with_context(|| format!("`{choice}` is not a listed option"))?;
            &groups[index - 1]
        }
    };

    let id = client
        .project_add(&NewProject {
            group_id: group.id,
            name: name.clone(),
            description: None,
        })
        .await?;
    println!("created \"{name}\" in {}", group.name);
    Ok((id, name))
}

fn describe(project: &Project) -> String {
    match &project.description {
        Some(description) if !description.is_empty() => {
            format!("{} — {description}", project.name)
        }
        _ => project.name.clone(),
    }
}

fn ask(lines: &mut impl Iterator<Item = std::io::Result<String>>, prompt: &str) -> Result<String> {
    print!("{prompt}: ");
    std::io::stdout().flush().ok();
    let line = lines
        .next()
        .transpose()
        .context("read stdin")?
        .unwrap_or_default();
    Ok(line.trim().to_string())
}
