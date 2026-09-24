//! `GET /api/typegen` — superuser-only download of the same `.d.ts`
//! `cratebase typegen` and the `--dev` `CB_TYPEGEN_OUT` watch produce
//! (`crate::typegen::generate`), so the CLI, the HTTP surface and the
//! dev watch never disagree about the generated shape.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::app::App;
use crate::extract::RequireSuperuser;

pub fn router() -> Router<App> {
    Router::new().route("/typegen", get(download))
}

async fn download(State(app): State<App>, _su: RequireSuperuser) -> Response {
    let snapshot = app.db().collections.all();
    let generated = crate::typegen::generate(snapshot.all.iter().map(|c| c.as_ref()));
    (
        [
            (
                header::CONTENT_TYPE,
                "application/typescript; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"cratebase-types.d.ts\"".to_string(),
            ),
        ],
        generated,
    )
        .into_response()
}
