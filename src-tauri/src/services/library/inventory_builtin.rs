//! Read-only matching for bundled recipes. A fixed source is not a claim of latest version.
use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuiltinRecipe {
    pub recipe_id: String,
    pub content: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    /// Repository-relative skill directory, not the local installation directory.
    pub directory: Option<String>,
}
fn valid_repo_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && ![".", ".."].contains(&value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
fn validate(kind: &ResourceKind, recipes: &[BuiltinRecipe]) -> Result<(), AppError> {
    if *kind == ResourceKind::Mcp
        || recipes.is_empty()
        || recipes.len() > 32
        || recipes
            .iter()
            .map(|recipe| &recipe.recipe_id)
            .collect::<BTreeSet<_>>()
            .len()
            != recipes.len()
    {
        return Err(error("inventorySelectionInvalid"));
    }
    let mut bytes = 0usize;
    for recipe in recipes {
        if recipe.recipe_id.is_empty() || recipe.recipe_id.len() > 128 {
            return Err(error("inventorySelectionInvalid"));
        }
        match kind {
            ResourceKind::Instructions => {
                let content = recipe
                    .content
                    .as_ref()
                    .ok_or_else(|| error("inventorySelectionInvalid"))?;
                bytes = bytes.saturating_add(content.len());
                if content.len() > 1024 * 1024
                    || recipe.repo_owner.is_some()
                    || recipe.repo_name.is_some()
                    || recipe.directory.is_some()
                {
                    return Err(error("inventorySelectionInvalid"));
                }
            }
            ResourceKind::Skills => {
                if recipe.content.is_some()
                    || !recipe.repo_owner.as_deref().is_some_and(valid_repo_part)
                    || !recipe.repo_name.as_deref().is_some_and(valid_repo_part)
                {
                    return Err(error("inventorySelectionInvalid"));
                }
                let directory = recipe
                    .directory
                    .as_deref()
                    .ok_or_else(|| error("inventorySelectionInvalid"))?;
                if directory.len() > 1024 || directory.contains('\\') {
                    return Err(error("inventorySelectionInvalid"));
                }
                if !directory.is_empty() {
                    super::super::skill_artifact::safe_relative(directory)?;
                }
            }
            ResourceKind::Mcp => unreachable!("rejected above"),
        }
    }
    if bytes > 4 * 1024 * 1024 {
        return Err(error("inventoryLimit"));
    }
    Ok(())
}
fn same_repository(recipe: &BuiltinRecipe, row: &InstalledSkill) -> bool {
    recipe
        .repo_owner
        .as_deref()
        .zip(row.repo_owner.as_deref())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && recipe
            .repo_name
            .as_deref()
            .zip(row.repo_name.as_deref())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
}
fn same_origin_directory(recipe: &BuiltinRecipe, row: &InstalledSkill) -> bool {
    if !same_repository(recipe, row) {
        return false;
    }
    let view = super::super::skill_actions::provenance_view(row);
    if view["sourceKind"] != "repository" {
        return false;
    }
    let Some(raw) = view["sourceUrl"].as_str() else {
        return false;
    };
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let Some(owner) = row.repo_owner.as_deref() else {
        return false;
    };
    let Some(name) = row.repo_name.as_deref() else {
        return false;
    };
    let prefix = format!("/{owner}/{name}/");
    let Some(tail) = url.path().strip_prefix(&prefix).and_then(|tail| {
        tail.strip_prefix("blob/")
            .or_else(|| tail.strip_prefix("tree/"))
    }) else {
        return false;
    };
    let path = if view["commitSha"].as_str().is_some() {
        tail.split_once('/').map(|(_, path)| path)
    } else {
        view["trackingRef"]
            .as_str()
            .and_then(|reference| tail.strip_prefix(&format!("{reference}/")))
    };
    let Some(path) = path else {
        return false;
    };
    let relative = recipe.directory.as_deref().unwrap_or_default();
    let expected = if relative.is_empty() {
        "SKILL.md".to_string()
    } else {
        format!("{relative}/SKILL.md")
    };
    path == expected
}
fn verified_fixed_skill(db: &Database, row: &InstalledSkill) -> bool {
    let view = super::super::skill_actions::provenance_view(row);
    view["commitSha"]
        .as_str()
        .is_some_and(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        && super::super::skill_resource::artifact_for(db, &row.id).is_ok()
}
fn native_candidates(
    db: &Database,
    kind: &ResourceKind,
    id: &str,
    epoch: u64,
) -> Result<Vec<Candidate>, AppError> {
    let (candidates, signature, sources) = {
        let mut tickets = pending()
            .lock()
            .map_err(|_| error("inventoryUnavailable"))?;
        prune(&mut tickets);
        let ticket = tickets.get(id).ok_or_else(|| error("inventoryExpired"))?;
        details::verify_context(db, ticket, epoch)?;
        if ticket.kind != *kind {
            return Err(error("inventorySelectionInvalid"));
        }
        (
            ticket.candidates.values().cloned().collect::<Vec<_>>(),
            ticket.source_signature.clone(),
            ticket.sources.clone(),
        )
    };
    if sources
        .iter()
        .any(|source| ["unreadable", "unsupported"].contains(&source.state.as_str()))
    {
        return Err(error("inventorySourceUnreadable"));
    }
    // A whole-inventory conclusion also rechecks empty directories and legacy sources.
    // This is read-only; no new ticket is retained and nothing is imported or executed.
    let fresh = readers::collect(db, kind.clone()).map_err(|_| error("inventoryChanged"))?;
    if readers::scan_signature(&fresh)? != signature {
        return Err(error("inventoryChanged"));
    }
    for candidate in &candidates {
        verify(db, candidate).map_err(|_| error("inventoryChanged"))?;
    }
    Ok(candidates)
}
pub fn match_builtin_recipes(
    db: &Database,
    kind: ResourceKind,
    recipes: &[BuiltinRecipe],
    inventory_ticket_id: Option<&str>,
    expected_epoch: u64,
) -> Result<Vec<McpRecipeMatch>, AppError> {
    let _io = crate::config_io::lock();
    super::super::catalog::verify_epoch(db, expected_epoch)?;
    validate(&kind, recipes)?;
    let native = inventory_ticket_id
        .map(|id| native_candidates(db, &kind, id, expected_epoch))
        .transpose()?
        .unwrap_or_default();
    let snapshot = db.library_snapshot()?;
    let skills = if kind == ResourceKind::Skills {
        db.get_all_installed_skills()?
    } else {
        Default::default()
    };
    let verified: BTreeMap<_, _> = skills
        .values()
        .map(|row| (row.id.clone(), verified_fixed_skill(db, row)))
        .collect();
    recipes.iter().map(|recipe|{
        let mut result=McpRecipeMatch{recipe_id:recipe.recipe_id.clone(),exact_resource_ids:vec![],native_entry_ids:vec![],similar_resource_ids:vec![],similar_native_entry_ids:vec![]};
        match kind {
            ResourceKind::Instructions => {
                for row in snapshot.definitions.instructions.values() {
                    if Some(&row.content)==recipe.content.as_ref(){result.exact_resource_ids.push(row.id.clone());}
                    else if row.id.eq_ignore_ascii_case(&recipe.recipe_id)||row.name.eq_ignore_ascii_case(&recipe.recipe_id){result.similar_resource_ids.push(row.id.clone());}
                }
            }
            ResourceKind::Skills => {
                for row in skills.values() {
                    if same_origin_directory(recipe,row)&&verified.get(&row.id)==Some(&true){result.exact_resource_ids.push(row.id.clone());}
                    else if same_origin_directory(recipe,row)||row.id.eq_ignore_ascii_case(&recipe.recipe_id)||row.name.eq_ignore_ascii_case(&recipe.recipe_id)||row.directory.eq_ignore_ascii_case(&recipe.recipe_id){result.similar_resource_ids.push(row.id.clone());}
                }
            }
            ResourceKind::Mcp => unreachable!("validated kind"),
        }
        for candidate in &native {
            if candidate.view.matched_resource_ids.iter().any(|id|result.exact_resource_ids.contains(id)){result.native_entry_ids.push(candidate.view.entry_id.clone());}
            else {
                let similar_content=matches!(&candidate.definition,Definition::Instructions{content} if Some(content)==recipe.content.as_ref());
                let similar_name=candidate.view.name.eq_ignore_ascii_case(&recipe.recipe_id)||candidate.view.directory.as_deref().is_some_and(|directory|directory.eq_ignore_ascii_case(&recipe.recipe_id));
                let linked_other=candidate.view.matched_resource_ids.iter().any(|id|result.similar_resource_ids.contains(id));
                if similar_content||similar_name||linked_other{result.similar_native_entry_ids.push(candidate.view.entry_id.clone());}
            }
        }
        Ok(result)
    }).collect()
}
