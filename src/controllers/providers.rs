use std::collections::HashMap;

use actix_web::{HttpResponse, get, post, web};
use askama::Template;
use futures_util::{StreamExt, stream};
use serde::Deserialize;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::model::{self, ModelRow};
use crate::models::provider::{self, Provider, ProviderForm, ProviderRow};
use crate::services::probes::ProbeView;
use crate::services::router;
use crate::views::{self, Nav, Page, flash};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index)
        .service(create)
        .service(show)
        .service(update)
        .service(destroy)
        .service(add_models)
        .service(import_models)
        .service(toggle_model)
        .service(delete_model)
        .service(test_model)
        .service(test_provider);
}

#[derive(Template)]
#[template(path = "providers.html")]
struct ProvidersView {
    page: Page,
    providers: Vec<ProviderRow>,
}

#[derive(Template)]
#[template(path = "provider.html")]
struct ProviderView {
    page: Page,
    provider: Provider,
    models: Vec<ModelRow>,
    /// Latest test result per model id.
    probes: HashMap<i64, ProbeView>,
}

#[get("/providers")]
async fn index(admin: Admin, state: web::Data<AppState>) -> WebResult<HttpResponse> {
    views::render(ProvidersView {
        page: Page::new(&admin.0, "Providers", Nav::Providers),
        providers: provider::list(&state.db).await?,
    })
}

#[post("/providers")]
async fn create(
    admin: Admin,
    state: web::Data<AppState>,
    form: web::Form<ProviderForm>,
) -> WebResult<HttpResponse> {
    if let Err(msg) = form.validate() {
        flash(&admin.0, false, msg);
        return Ok(views::redirect("/providers"));
    }
    match provider::create(&state.db, &form).await {
        Ok(id) => {
            flash(
                &admin.0,
                true,
                "Provider added. Import or add its models below.",
            );
            Ok(views::redirect(&format!("/providers/{id}")))
        }
        Err(e) if provider::is_duplicate(&e) => {
            flash(&admin.0, false, "A provider with that name already exists.");
            Ok(views::redirect("/providers"))
        }
        Err(e) => Err(e.into()),
    }
}

#[get("/providers/{id}")]
async fn show(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let provider = provider::get(&state.db, *id).await?;
    let models = model::list_for_provider(&state.db, provider.id).await?;
    views::render(ProviderView {
        page: Page::new(&admin.0, provider.name.clone(), Nav::Providers),
        probes: state.probes.view(models.iter().map(|m| m.id)),
        models,
        provider,
    })
}

#[post("/providers/{id}")]
async fn update(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
    form: web::Form<ProviderForm>,
) -> WebResult<HttpResponse> {
    let to = format!("/providers/{id}");
    if let Err(msg) = form.validate() {
        flash(&admin.0, false, msg);
        return Ok(views::redirect(&to));
    }
    match provider::update(&state.db, *id, &form).await {
        Ok(()) => flash(&admin.0, true, "Provider saved."),
        Err(e) if provider::is_duplicate(&e) => {
            flash(&admin.0, false, "A provider with that name already exists.")
        }
        Err(e) => return Err(e.into()),
    }
    Ok(views::redirect(&to))
}

#[post("/providers/{id}/delete")]
async fn destroy(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    provider::delete(&state.db, *id).await?;
    flash(
        &admin.0,
        true,
        "Provider deleted, with its models and combo entries.",
    );
    Ok(views::redirect("/providers"))
}

#[derive(Deserialize)]
struct ModelsForm {
    /// One model id per line (commas also work).
    names: String,
}

#[post("/providers/{id}/models")]
async fn add_models(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
    form: web::Form<ModelsForm>,
) -> WebResult<HttpResponse> {
    let names: Vec<String> = form
        .names
        .split([',', '\n'])
        .map(|s| s.trim().to_string())
        .collect();
    let added = model::add_many(&state.db, *id, &names).await?;
    flash(&admin.0, true, format!("{added} model(s) added."));
    Ok(views::redirect(&format!("/providers/{id}")))
}

#[post("/providers/{id}/import")]
async fn import_models(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let provider = provider::get(&state.db, *id).await?;
    match state
        .upstream
        .fetch_models(&provider.base_url, &provider.api_key)
        .await
    {
        Ok(names) => {
            let added = model::add_many(&state.db, provider.id, &names).await?;
            flash(
                &admin.0,
                true,
                format!("Provider lists {} model(s); {added} new.", names.len()),
            );
        }
        Err(e) => flash(&admin.0, false, format!("Could not list models: {e}")),
    }
    Ok(views::redirect(&format!("/providers/{id}")))
}

#[post("/models/{id}/toggle")]
async fn toggle_model(
    _admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let provider_id = model::toggle(&state.db, *id).await?;
    Ok(views::redirect(&format!("/providers/{provider_id}")))
}

#[post("/models/{id}/delete")]
async fn delete_model(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let provider_id = model::delete(&state.db, *id).await?;
    flash(&admin.0, true, "Model removed.");
    Ok(views::redirect(&format!("/providers/{provider_id}")))
}

/// How many models "Test all" checks at once.
const TEST_CONCURRENCY: usize = 6;

#[post("/models/{id}/test")]
async fn test_model(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let target = router::target_for_model(&state.db, *id).await?;
    let probe = state.upstream.probe(&target).await;
    let message = if probe.ok {
        format!(
            "{} answered in {}.",
            target.model_name,
            views::ms(probe.latency_ms as f64)
        )
    } else {
        format!("{} failed: {}", target.model_name, probe.detail)
    };
    flash(&admin.0, probe.ok, message);
    state.probes.store(target.model_id, probe);
    let provider_id = model::provider_of(&state.db, *id).await?;
    Ok(views::redirect(&format!(
        "/providers/{provider_id}#model-{id}"
    )))
}

#[post("/providers/{id}/test")]
async fn test_provider(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let targets = router::targets_for_provider(&state.db, *id).await?;
    let total = targets.len();
    let results: Vec<_> = stream::iter(targets)
        .map(|t| {
            let state = state.clone();
            async move {
                let probe = state.upstream.probe(&t).await;
                (t.model_id, probe)
            }
        })
        .buffer_unordered(TEST_CONCURRENCY)
        .collect()
        .await;
    let passed = results.iter().filter(|(_, p)| p.ok).count();
    for (model_id, probe) in results {
        state.probes.store(model_id, probe);
    }
    flash(
        &admin.0,
        passed == total,
        format!("{passed} of {total} model(s) answered."),
    );
    Ok(views::redirect(&format!("/providers/{id}")))
}
