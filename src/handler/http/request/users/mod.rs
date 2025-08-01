// Copyright 2025 OpenObserve Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{collections::HashMap, io::Error, sync::Arc};

use actix_web::{
    HttpRequest, HttpResponse, cookie, delete, get,
    http::{self},
    post, put, web,
};
use actix_web_httpauth::extractors::basic::BasicAuth;
use config::{
    Config, get_config,
    meta::user::UserRole,
    utils::{base64, json},
};
use serde::Serialize;
#[cfg(feature = "enterprise")]
use {
    crate::common::utils::auth::check_permissions,
    crate::service::self_reporting::audit,
    config::utils::time::now_micros,
    o2_dex::config::get_config as get_dex_config,
    o2_enterprise::enterprise::common::auditor::{AuditMessage, Protocol, ResponseMeta},
    o2_openfga::config::get_config as get_openfga_config,
};

use crate::{
    common::{
        meta::{
            self,
            user::{
                AuthTokens, PostUserRequest, RolesResponse, SignInResponse, SignInUser, UpdateUser,
                UserOrgRole, UserRequest, UserRoleRequest, get_roles,
            },
        },
        utils::auth::{UserEmail, generate_presigned_url, is_valid_email},
    },
    service::users,
};

pub mod service_accounts;

/// ListUsers
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"list"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "UserList",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
      ),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = UserList),
    )
)]
#[get("/{org_id}/users")]
pub async fn list(
    org_id: web::Path<String>,
    user_email: UserEmail,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let org_id = org_id.into_inner();
    let query = web::Query::<HashMap<String, String>>::from_query(req.query_string()).unwrap();
    let list_all = match query.get("list_all") {
        Some(v) => v.parse::<bool>().unwrap_or(false),
        None => false,
    };

    let mut _user_list_from_rbac = None;

    #[cfg(feature = "enterprise")]
    // Check if user has access to get users
    if get_openfga_config().enabled
        && check_permissions(
            Some(format!("_all_{org_id}")),
            &org_id,
            &user_email.user_id,
            "users",
            "GET",
            "",
        )
        .await
    {
        _user_list_from_rbac = Some(vec![]);
    }

    users::list_users(
        &user_email.user_id,
        &org_id,
        None,
        _user_list_from_rbac,
        list_all,
    )
    .await
}

/// CreateUser
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"create"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "UserSave",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
    ),
    request_body(content = PostUserRequest, description = "User data", content_type = "application/json"),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = HttpResponse),
    )
)]
#[post("/{org_id}/users")]
pub async fn save(
    org_id: web::Path<String>,
    user: web::Json<PostUserRequest>,
    user_email: UserEmail,
) -> Result<HttpResponse, Error> {
    let org_id = org_id.into_inner();
    let initiator_id = user_email.user_id;
    let mut user = UserRequest::from(&user.into_inner());
    user.email = user.email.trim().to_lowercase();

    let bad_req_msg = if user.password.len() < 8 {
        Some("Password must be at least 8 characters long")
    } else if user.role.base_role == UserRole::Root {
        Some("Not allowed")
    } else if !is_valid_email(user.email.as_str()) {
        Some("Invalid Email address")
    } else {
        None
    };
    if let Some(msg) = bad_req_msg {
        return Ok(
            HttpResponse::BadRequest().json(meta::http::HttpResponse::error(
                http::StatusCode::BAD_REQUEST,
                msg.to_string(),
            )),
        );
    }

    #[cfg(not(feature = "enterprise"))]
    {
        user.role.base_role = UserRole::Admin;
    }
    users::post_user(&org_id, user, &initiator_id).await
}

/// UpdateUser
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"update"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "UserUpdate",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("email_id" = String, Path, description = "User's email id"),
    ),
    request_body(content = UpdateUser, description = "User data", content_type = "application/json"),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = HttpResponse),
    )
)]
#[put("/{org_id}/users/{email_id}")]
pub async fn update(
    params: web::Path<(String, String)>,
    user: web::Json<UpdateUser>,
    user_email: UserEmail,
) -> Result<HttpResponse, Error> {
    let (org_id, email_id) = params.into_inner();
    let email_id = email_id.trim().to_lowercase();
    #[cfg(not(feature = "enterprise"))]
    let mut user = user.into_inner();
    #[cfg(feature = "enterprise")]
    let user = user.into_inner();
    if user.eq(&UpdateUser::default()) {
        return Ok(
            HttpResponse::BadRequest().json(meta::http::HttpResponse::error(
                http::StatusCode::BAD_REQUEST,
                "Please specify appropriate fields to update user",
            )),
        );
    }
    if user.change_password
        && user
            .new_password
            .as_deref()
            .is_some_and(|pass| pass.len() < 8)
    {
        return Ok(
            HttpResponse::BadRequest().json(meta::http::HttpResponse::error(
                http::StatusCode::BAD_REQUEST,
                "Password must be at least 8 characters long".to_string(),
            )),
        );
    }
    #[cfg(not(feature = "enterprise"))]
    {
        user.role = Some(UserRoleRequest {
            role: UserRole::Admin.to_string(),
            custom: None,
        });
    }
    let initiator_id = &user_email.user_id;
    let self_update = user_email.user_id.eq(&email_id);
    users::update_user(&org_id, &email_id, self_update, initiator_id, user).await
}

/// AddUserToOrganization
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"create"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "AddUserToOrg",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("email_id" = String, Path, description = "User's email id"),
    ),
    request_body(content = UserRoleRequest, description = "User role", content_type = "application/json"),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = HttpResponse),
    )
)]
#[post("/{org_id}/users/{email_id}")]
pub async fn add_user_to_org(
    params: web::Path<(String, String)>,
    role: web::Json<UserRoleRequest>,
    user_email: UserEmail,
) -> Result<HttpResponse, Error> {
    let (org_id, email_id) = params.into_inner();
    let initiator_id = user_email.user_id;
    let role = UserOrgRole::from(&role.into_inner());
    users::add_user_to_org(&org_id, &email_id, role, &initiator_id).await
}

fn _prepare_cookie<'a, T: Serialize + ?Sized, E: Into<cookie::Expiration>>(
    conf: &Arc<Config>,
    cookie_name: &'a str,
    token_struct: &T,
    cookie_expiry: E,
) -> cookie::Cookie<'a> {
    let tokens = json::to_string(token_struct).unwrap();
    let tokens = base64::encode(&tokens);
    let mut auth_cookie = cookie::Cookie::new(cookie_name, tokens);
    auth_cookie.set_expires(cookie_expiry.into());
    auth_cookie.set_http_only(true);
    auth_cookie.set_secure(conf.auth.cookie_secure_only);
    auth_cookie.set_path("/");
    if conf.auth.cookie_same_site_lax {
        auth_cookie.set_same_site(cookie::SameSite::Lax);
    } else {
        auth_cookie.set_same_site(cookie::SameSite::None);
    }
    auth_cookie
}
/// RemoveUserFromOrganization
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"delete"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "RemoveUserFromOrg",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("email_id" = String, Path, description = "User name"),
      ),
    responses(
        (status = 200, description = "Success",  content_type = "application/json", body = HttpResponse),
        (status = 404, description = "NotFound", content_type = "application/json", body = HttpResponse),
    )
)]
#[delete("/{org_id}/users/{email_id}")]
pub async fn delete(
    path: web::Path<(String, String)>,
    user_email: UserEmail,
) -> Result<HttpResponse, Error> {
    let (org_id, email_id) = path.into_inner();
    let initiator_id = user_email.user_id;
    users::remove_user_from_org(&org_id, &email_id, &initiator_id).await
}

/// AuthenticateUser
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"update"}#
#[utoipa::path(
context_path = "/auth",
    tag = "Auth",
    operation_id = "UserLoginCheck",
    request_body(content = SignInUser, description = "User login", content_type = "application/json"),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = SignInResponse),
    )
)]
#[post("/login")]
pub async fn authentication(
    auth: Option<web::Json<SignInUser>>,
    _req: HttpRequest,
) -> Result<HttpResponse, Error> {
    #[cfg(feature = "enterprise")]
    let native_login_enabled = get_dex_config().native_login_enabled;
    #[cfg(not(feature = "enterprise"))]
    let native_login_enabled = true;

    if !native_login_enabled {
        return Ok(HttpResponse::Forbidden().json("Not Supported"));
    }

    // Until decoding the token or body, we can not know the the user_email
    #[cfg(feature = "enterprise")]
    let mut audit_message = AuditMessage {
        user_email: "".to_string(),
        org_id: "".to_string(),
        _timestamp: now_micros(),
        protocol: Protocol::Http,
        response_meta: ResponseMeta {
            http_method: "POST".to_string(),
            http_path: "/auth/login".to_string(),
            http_body: "".to_string(),
            http_query_params: _req.query_string().to_string(),
            http_response_code: 200,
            error_msg: None,
            trace_id: None,
        },
    };

    let mut resp = SignInResponse::default();
    let auth = match auth {
        Some(auth) => {
            let mut auth = auth.into_inner();
            auth.name = auth.name.to_lowercase();
            auth
        }
        None => {
            // get Authorization header from request
            #[cfg(feature = "enterprise")]
            {
                let auth_header = _req.headers().get("Authorization");
                if let Some(auth_header) = auth_header {
                    if let Some((name, password)) =
                        o2_dex::service::auth::get_user_from_token(auth_header.to_str().unwrap())
                    {
                        SignInUser { name, password }
                    } else {
                        audit_unauthorized_error(audit_message).await;
                        return unauthorized_error(resp);
                    }
                } else {
                    audit_unauthorized_error(audit_message).await;
                    return unauthorized_error(resp);
                }
            }
            #[cfg(not(feature = "enterprise"))]
            {
                return unauthorized_error(resp);
            }
        }
    };

    #[cfg(feature = "enterprise")]
    {
        audit_message.user_email = auth.name.clone();
    }

    #[cfg(feature = "enterprise")]
    {
        if get_dex_config().root_only_login && !crate::common::utils::auth::is_root_user(&auth.name)
        {
            audit_unauthorized_error(audit_message).await;
            return unauthorized_error(resp);
        }
    }
    match crate::handler::http::auth::validator::validate_user(&auth.name, &auth.password).await {
        Ok(v) => {
            if v.is_valid {
                resp.status = true;
            } else {
                #[cfg(feature = "enterprise")]
                audit_unauthorized_error(audit_message).await;
                return unauthorized_error(resp);
            }
        }
        Err(_e) => {
            #[cfg(feature = "enterprise")]
            audit_unauthorized_error(audit_message).await;
            return unauthorized_error(resp);
        }
    };
    if resp.status {
        let cfg = get_config();

        let access_token = format!(
            "Basic {}",
            base64::encode(&format!("{}:{}", auth.name, auth.password))
        );
        let tokens = json::to_string(&AuthTokens {
            access_token,
            refresh_token: "".to_string(),
        })
        .unwrap();

        let tokens = base64::encode(&tokens);
        let mut auth_cookie = cookie::Cookie::new("auth_tokens", tokens);
        auth_cookie.set_expires(
            cookie::time::OffsetDateTime::now_utc()
                + cookie::time::Duration::seconds(cfg.auth.cookie_max_age),
        );
        auth_cookie.set_http_only(true);
        auth_cookie.set_secure(cfg.auth.cookie_secure_only);
        auth_cookie.set_path("/");
        if cfg.auth.cookie_same_site_lax {
            auth_cookie.set_same_site(cookie::SameSite::Lax);
        } else {
            auth_cookie.set_same_site(cookie::SameSite::None);
        }
        // audit the successful login
        #[cfg(feature = "enterprise")]
        audit(audit_message).await;
        Ok(HttpResponse::Ok().cookie(auth_cookie).json(resp))
    } else {
        #[cfg(feature = "enterprise")]
        audit_unauthorized_error(audit_message).await;
        unauthorized_error(resp)
    }
}

#[derive(serde::Deserialize)]
struct PresignedURLGenerator {
    #[serde(default = "default_exp_in")]
    exp_in: u32,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PresignedURLGeneratorResponse {
    url: String,
}

const fn default_exp_in() -> u32 {
    600
}

#[get("/presigned-url")]
pub async fn get_presigned_url(
    _req: HttpRequest,
    basic_auth: BasicAuth,
    query: web::Query<PresignedURLGenerator>,
) -> Result<HttpResponse, Error> {
    let cfg = get_config();
    let time = chrono::Utc::now().timestamp();
    let password_ext_salt = cfg.auth.ext_auth_salt.as_str();

    let base_url = format!("{}{}", cfg.common.web_url, cfg.common.base_uri);
    let url = generate_presigned_url(
        basic_auth.user_id(),
        basic_auth.password().unwrap(),
        password_ext_salt,
        &base_url,
        query.exp_in as i64,
        time,
    );

    let payload = PresignedURLGeneratorResponse { url };
    #[cfg(feature = "enterprise")]
    {
        let audit_message = AuditMessage {
            user_email: basic_auth.user_id().to_string(),
            org_id: "".to_string(),
            _timestamp: now_micros(),
            protocol: Protocol::Http,
            response_meta: ResponseMeta {
                http_method: "GET".to_string(),
                http_path: "/auth/presigned-url".to_string(),
                http_body: "".to_string(),
                http_query_params: _req.query_string().to_string(),
                http_response_code: 200,
                error_msg: None,
                trace_id: None,
            },
        };
        audit(audit_message).await;
    }
    Ok(HttpResponse::Ok().json(&payload))
}

#[get("/login")]
pub async fn get_auth(_req: HttpRequest) -> Result<HttpResponse, Error> {
    #[cfg(feature = "enterprise")]
    {
        use actix_web::http::header;
        use chrono::Utc;

        use crate::{
            common::meta::user::AuthTokensExt, handler::http::auth::validator::ID_TOKEN_HEADER,
        };

        let mut resp = SignInResponse::default();

        let query = web::Query::<std::collections::HashMap<String, String>>::from_query(
            _req.query_string(),
        )
        .unwrap();

        let mut request_time = None;
        let mut expires_in = 300;
        let mut req_ts = 0;

        let mut audit_message = AuditMessage {
            user_email: "".to_string(),
            org_id: "".to_string(),
            _timestamp: now_micros(),
            protocol: Protocol::Http,
            response_meta: ResponseMeta {
                http_method: "GET".to_string(),
                http_path: "/auth/login".to_string(),
                http_body: "".to_string(),
                // Don't include query string as it may contain the auth token
                http_query_params: "".to_string(),
                http_response_code: 302,
                error_msg: None,
                trace_id: None,
            },
        };

        let (name, password) = {
            let auth_header = if let Some(s) = query.get("auth") {
                match query.get("request_time") {
                    Some(req_time_str) => {
                        if let Ok(ts) = config::utils::time::parse_str_to_time(req_time_str) {
                            req_ts = ts.timestamp();
                        } else {
                            audit_unauthorized_error(audit_message).await;
                            return unauthorized_error(resp);
                        }
                        request_time = Some(req_time_str);
                    }
                    None => {
                        audit_unauthorized_error(audit_message).await;
                        return unauthorized_error(resp);
                    }
                };

                match query.get("exp_in") {
                    Some(exp_in_str) => {
                        expires_in = exp_in_str.parse::<i64>().unwrap();
                    }
                    None => {
                        audit_unauthorized_error(audit_message).await;
                        return unauthorized_error(resp);
                    }
                };
                if Utc::now().timestamp() - req_ts > expires_in {
                    audit_unauthorized_error(audit_message).await;
                    return unauthorized_error(resp);
                }
                format!("q_auth {s}")
            } else if let Some(auth_header) = _req.headers().get("Authorization") {
                match auth_header.to_str() {
                    Ok(auth_header_str) => auth_header_str.to_string(),
                    Err(_) => {
                        audit_unauthorized_error(audit_message).await;
                        return unauthorized_error(resp);
                    }
                }
            } else {
                audit_unauthorized_error(audit_message).await;
                return unauthorized_error(resp);
            };

            use o2_dex::service::auth::get_user_from_token;

            use crate::handler::http::auth::validator::{
                validate_user, validate_user_for_query_params,
            };

            let (name, password) = if let Some((name, password)) = get_user_from_token(&auth_header)
            {
                let token_validation_response = match request_time {
                    Some(req_ts) => {
                        log::debug!("Validating user for query params");
                        validate_user_for_query_params(&name, &password, Some(req_ts), expires_in)
                            .await
                    }
                    None => {
                        log::debug!("Validating user for basic auth header");
                        validate_user(&name, &password).await
                    }
                };

                audit_message.user_email = name.clone();
                match token_validation_response {
                    Ok(v) => {
                        if v.is_valid {
                            resp.status = true;
                            (name, password)
                        } else {
                            audit_unauthorized_error(audit_message).await;
                            return unauthorized_error(resp);
                        }
                    }
                    Err(_) => {
                        audit_unauthorized_error(audit_message).await;
                        return unauthorized_error(resp);
                    }
                }
            } else {
                audit_unauthorized_error(audit_message).await;
                return unauthorized_error(resp);
            };
            (name, password)
        };

        if resp.status {
            let cfg = get_config();
            let id_token = config::utils::json::json!({
                "email": name,
                "name": name,
            });
            let cookie_name = "auth_tokens";
            let auth_cookie = if req_ts == 0 {
                let access_token = format!(
                    "Basic {}",
                    base64::encode(&format!("{}:{}", &name, &password))
                );
                let tokens = AuthTokens {
                    access_token,
                    refresh_token: "".to_string(),
                };
                let expiry = cookie::time::OffsetDateTime::now_utc()
                    + cookie::time::Duration::seconds(cfg.auth.cookie_max_age);

                log::debug!("Setting cookie for user: {name} - {cookie_name}");
                _prepare_cookie(&cfg, cookie_name, &tokens, expiry)
            } else {
                let cookie_name = "auth_ext";
                let auth_ext = format!(
                    "{} {}",
                    cookie_name,
                    base64::encode(&format!("{}:{}", &name, &password))
                );

                let tokens = AuthTokensExt {
                    auth_ext,
                    refresh_token: "".to_string(),
                    request_time: req_ts,
                    expires_in,
                };
                let expiry = cookie::time::OffsetDateTime::now_utc()
                    + cookie::time::Duration::seconds(req_ts);

                log::debug!("Setting cookie for user: {name} - {cookie_name}");
                _prepare_cookie(&cfg, cookie_name, &tokens, expiry)
            };

            let url = format!(
                "{}{}/web/cb#id_token={}.{}",
                cfg.common.web_url,
                cfg.common.base_uri,
                ID_TOKEN_HEADER,
                base64::encode(&id_token.to_string())
            );
            audit_message._timestamp = Utc::now().timestamp_micros();
            audit(audit_message).await;
            Ok(HttpResponse::Found()
                .append_header((header::LOCATION, url))
                .cookie(auth_cookie)
                .json(resp))
        } else {
            audit_unauthorized_error(audit_message).await;
            unauthorized_error(resp)
        }
    }

    #[cfg(not(feature = "enterprise"))]
    {
        Ok(HttpResponse::Forbidden().json("Not Supported"))
    }
}

/// ListUserRoles
///
/// #{"ratelimit_module":"Users", "ratelimit_module_operation":"list"}#
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "UserRoles",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
      ),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = UserList),
    )
)]
#[get("/{org_id}/users/roles")]
pub async fn list_roles(_org_id: web::Path<String>) -> Result<HttpResponse, Error> {
    let roles = get_roles()
        .iter()
        .filter_map(check_role_available)
        .collect::<Vec<RolesResponse>>();

    Ok(HttpResponse::Ok().json(roles))
}

fn check_role_available(role: &UserRole) -> Option<RolesResponse> {
    if role.eq(&UserRole::Root) || role.eq(&UserRole::ServiceAccount) {
        None
    } else {
        #[cfg(feature = "enterprise")]
        if !get_openfga_config().enabled && role.ne(&UserRole::Admin) {
            return None;
        }
        Some(RolesResponse {
            label: role.get_label(),
            value: role.to_string(),
        })
    }
}

fn unauthorized_error(mut resp: SignInResponse) -> Result<HttpResponse, Error> {
    resp.status = false;
    resp.message = "Invalid credentials".to_string();
    Ok(HttpResponse::Unauthorized().json(resp))
}

#[cfg(feature = "enterprise")]
async fn audit_unauthorized_error(mut audit_message: AuditMessage) {
    use chrono::Utc;

    audit_message._timestamp = Utc::now().timestamp_micros();
    audit_message.response_meta.http_response_code = 401;
    // Even if the user_email of audit_message is not set, still the event should be audited
    audit(audit_message).await;
}

/// ListUserInvitations
#[cfg(feature = "cloud")]
#[utoipa::path(
    context_path = "/api",
    tag = "Users",
    operation_id = "UserInvitations",
    security(
        ("Authorization"= [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
      ),
    responses(
        (status = 200, description = "Success", content_type = "application/json", body = UserList),
    )
)]
#[get("/invites")]
pub async fn list_invitations(user_email: UserEmail) -> Result<HttpResponse, Error> {
    let user_id = user_email.user_id.as_str();
    users::list_user_invites(user_id).await
}

#[cfg(not(feature = "cloud"))]
#[get("/invites")]
pub async fn list_invitations(_user_email: UserEmail) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Forbidden().json("Not Supported"))
}

#[cfg(test)]
mod tests {
    use actix_web::{App, test};
    use actix_web_httpauth::headers::authorization::Basic;

    use super::*;

    #[tokio::test]
    async fn test_get_presigned_url() {
        let app = test::init_service(App::new().service(get_presigned_url)).await;

        let auth = Basic::new("username", Some("password"));
        let req = test::TestRequest::get()
            .uri("/presigned-url")
            .append_header((actix_web::http::header::AUTHORIZATION, auth))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), http::StatusCode::OK);

        let body = test::read_body(resp).await;
        let response_body: PresignedURLGeneratorResponse = serde_json::from_slice(&body).unwrap();

        assert!(!response_body.url.is_empty());
    }
}
