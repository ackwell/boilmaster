use axum::{
	extract::{Request, State},
	http::{header, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
};
use axum_extra::{
	headers::{authorization::Basic, Authorization},
	TypedHeader,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct BasicAuth {
	username: String,
	password: String,
}

pub async fn basic_auth(
	State(expected): State<BasicAuth>,
	authorization: Option<TypedHeader<Authorization<Basic>>>,
	request: Request,
	next: Next,
) -> Response {
	let authenticated = authorization.map_or(false, |TypedHeader(auth)| {
		auth.username() == expected.username && auth.password() == expected.password
	});

	match authenticated {
		true => next.run(request).await,
		false => {
			// TypedHeader seems to just... not have this? eh?
			(
				StatusCode::UNAUTHORIZED,
				[(
					header::WWW_AUTHENTICATE,
					"Basic realm=\"boilmaster\", charset=\"UTF-8\"",
				)],
			)
				.into_response()
		}
	}
}
