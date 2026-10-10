use actix_web::{HttpResponse, get, post, web};
use askama::Template;
use serde::Deserialize;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::combo::{self, Combo, ComboForm, ComboRow, Direction, EntryRow};
use crate::models::model::{self, ModelRow};
use crate::models::provider;
use crate::views::{self, Nav, Page, flash};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index)
        .service(create)
        .service(show)
        .service(update)
        .service(destroy)
        .service(add_entry)
        .service(move_entry)
        .service(remove_entry);
}

#[derive(Template)]
#[template(path = "combos.html")]
struct CombosView {
    page: Page,
    combos: Vec<ComboRow>,
}

#[derive(Template)]
#[template(path = "combo.html")]
struct ComboView {
    page: Page,
    combo: Combo,
    entries: Vec<EntryRow>,
    /// Models not yet in the chain.
    available: Vec<ModelRow>,
}

#[get("/combos")]
async fn index(admin: Admin, state: web::Data<AppState>) -> WebResult<HttpResponse> {
    views::render(CombosView {
        page: Page::new(&admin.0, "Combos", Nav::Combos),
        combos: combo::list(&state.db).await?,
    })
}

#[post("/combos")]
async fn create(
    admin: Admin,
    state: web::Data<AppState>,
    form: web::Form<ComboForm>,
) -> WebResult<HttpResponse> {
    if let Err(msg) = form.validate() {
        flash(&admin.0, false, msg);
        return Ok(views::redirect("/combos"));
    }
    match combo::create(&state.db, &form).await {
        Ok(id) => {
            flash(
                &admin.0,
                true,
                "Combo created. Add models in the order they should be tried.",
            );
            Ok(views::redirect(&format!("/combos/{id}")))
        }
        Err(e) if provider::is_duplicate(&e) => {
            flash(&admin.0, false, "A combo with that name already exists.");
            Ok(views::redirect("/combos"))
        }
        Err(e) => Err(e.into()),
    }
}

#[get("/combos/{id}")]
async fn show(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    let combo = combo::get(&state.db, *id).await?;
    let entries = combo::entries(&state.db, combo.id).await?;
    let available = model::list_all(&state.db)
        .await?
        .into_iter()
        .filter(|m| !entries.iter().any(|e| e.model_id == m.id))
        .collect();
    views::render(ComboView {
        page: Page::new(&admin.0, combo.name.clone(), Nav::Combos),
        combo,
        entries,
        available,
    })
}

#[post("/combos/{id}")]
async fn update(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
    form: web::Form<ComboForm>,
) -> WebResult<HttpResponse> {
    let to = format!("/combos/{id}");
    if let Err(msg) = form.validate() {
        flash(&admin.0, false, msg);
        return Ok(views::redirect(&to));
    }
    match combo::update(&state.db, *id, &form).await {
        Ok(()) => flash(&admin.0, true, "Combo saved."),
        Err(e) if provider::is_duplicate(&e) => {
            flash(&admin.0, false, "A combo with that name already exists.")
        }
        Err(e) => return Err(e.into()),
    }
    Ok(views::redirect(&to))
}

#[post("/combos/{id}/delete")]
async fn destroy(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    combo::delete(&state.db, *id).await?;
    flash(&admin.0, true, "Combo deleted.");
    Ok(views::redirect("/combos"))
}

#[derive(Deserialize)]
struct EntryForm {
    model_id: i64,
}

#[post("/combos/{id}/entries")]
async fn add_entry(
    _admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
    form: web::Form<EntryForm>,
) -> WebResult<HttpResponse> {
    combo::add_entry(&state.db, *id, form.model_id).await?;
    Ok(views::redirect(&format!("/combos/{id}")))
}

#[derive(Deserialize)]
struct MoveForm {
    dir: Direction,
}

#[post("/combos/{id}/entries/{model_id}/move")]
async fn move_entry(
    _admin: Admin,
    state: web::Data<AppState>,
    path: web::Path<(i64, i64)>,
    form: web::Form<MoveForm>,
) -> WebResult<HttpResponse> {
    let (id, model_id) = *path;
    combo::move_entry(&state.db, id, model_id, form.dir).await?;
    Ok(views::redirect(&format!("/combos/{id}")))
}

#[post("/combos/{id}/entries/{model_id}/delete")]
async fn remove_entry(
    _admin: Admin,
    state: web::Data<AppState>,
    path: web::Path<(i64, i64)>,
) -> WebResult<HttpResponse> {
    let (id, model_id) = *path;
    combo::remove_entry(&state.db, id, model_id).await?;
    Ok(views::redirect(&format!("/combos/{id}")))
}
