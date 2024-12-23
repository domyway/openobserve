// Copyright 2024 OpenObserve Inc.
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

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};

use crate::{
    common::meta::http::HttpResponse as MetaHttpResponse,
    handler::http::models::folders::{
        CreateFolderRequestBody, CreateFolderResponseBody, ListFoldersResponseBody,
        UpdateFolderRequestBody,
    },
    service::folders::{self, FolderError},
};

impl From<FolderError> for HttpResponse {
    fn from(value: FolderError) -> Self {
        match value {
            FolderError::InfraError(err) => MetaHttpResponse::internal_error(err),
            FolderError::MissingName => {
                MetaHttpResponse::bad_request("Folder name cannot be empty")
            }
            FolderError::UpdateDefaultFolder => {
                MetaHttpResponse::bad_request("Can't update default folder")
            }
            FolderError::DeleteWithDashboards => MetaHttpResponse::bad_request(
                "Dashboard folder contains dashboards, please move/delete dashboards from folder",
            ),
            FolderError::NotFound => MetaHttpResponse::not_found("Folder not found"),
            FolderError::PermittedFoldersMissingUser => MetaHttpResponse::forbidden(""),
            FolderError::PermittedFoldersValidator(err) => MetaHttpResponse::forbidden(err),
        }
    }
}

/// CreateFolder
#[utoipa::path(
    context_path = "/api",
    tag = "Folders",
    operation_id = "CreateFolder",
    security(
        ("Authorization" = [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
    ),
    request_body(
        content = CreateFolderRequestBody,
        description = "Folder details",
        example = json!({
            "name": "Infrastructure",
            "description": "Traffic patterns and network performance of the infrastructure",
        }),
    ),
    responses(
        (status = StatusCode::OK, description = "Folder created", body = CreateFolderResponseBody),
        (status = StatusCode::INTERNAL_SERVER_ERROR, description = "Internal Server Error", body = HttpResponse),
    ),
)]
#[post("/{org_id}/folders")]
pub async fn create_folder(
    path: web::Path<String>,
    body: web::Json<CreateFolderRequestBody>,
) -> impl Responder {
    let org_id = path.into_inner();
    let folder = body.into_inner().into();
    match folders::save_folder(&org_id, folder, false).await {
        Ok(folder) => {
            let body: CreateFolderResponseBody = folder.into();
            HttpResponse::Ok().json(body)
        }
        Err(err) => err.into(),
    }
}

/// UpdateFolder
#[utoipa::path(
    context_path = "/api",
    tag = "Folders",
    operation_id = "UpdateFolder",
    security(
        ("Authorization" = [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("folder_id" = String, Path, description = "Folder name"),
    ),
    request_body(
        content = Folder,
        description = "Folder details",
        example = json!({
            "title": "Infra",
            "description": "Traffic patterns and network performance of the infrastructure",
        }),
    ),
    responses(
        (status = StatusCode::OK, description = "Folder updated", body = HttpResponse),
        (status = StatusCode::INTERNAL_SERVER_ERROR, description = "Internal Server Error", body = HttpResponse),
    ),
)]
#[put("/{org_id}/folders/{folder_id}")]
pub async fn update_folder(
    path: web::Path<(String, String)>,
    body: web::Json<UpdateFolderRequestBody>,
) -> impl Responder {
    let (org_id, folder_id) = path.into_inner();
    let folder = body.into_inner().into();
    match folders::update_folder(&org_id, &folder_id, folder).await {
        Ok(_) => HttpResponse::Ok().body("Folder updated"),
        Err(err) => err.into(),
    }
}

/// ListFolders
#[utoipa::path(
    context_path = "/api",
    tag = "Folders",
    operation_id = "ListFolders",
    security(
        ("Authorization" = [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
    ),
    responses(
        (status = StatusCode::OK, body = ListFoldersResponseBody),
    ),
)]
#[get("/{org_id}/folders")]
#[allow(unused_variables)]
pub async fn list_folders(path: web::Path<String>, req: HttpRequest) -> impl Responder {
    let org_id = path.into_inner();

    #[cfg(not(feature = "enterprise"))]
    let user_id = None;

    #[cfg(feature = "enterprise")]
    let Ok(user_id) = req.headers().get("user_id").map(|v| v.to_str()).transpose() else {
        return HttpResponse::Forbidden().finish();
    };

    match folders::list_folders(&org_id, user_id).await {
        Ok(folders) => {
            let body: ListFoldersResponseBody = folders.into();
            HttpResponse::Ok().json(body)
        }
        Err(err) => err.into(),
    }
}

/// GetFolder
#[utoipa::path(
    context_path = "/api",
    tag = "Folders",
    operation_id = "GetFolder",
    security(
        ("Authorization" = [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("folder_id" = String, Path, description = "Folder ID"),
    ),
    responses(
        (status = StatusCode::OK, body = GetFolderResponseBody),
        (status = StatusCode::NOT_FOUND, description = "Folder not found", body = HttpResponse),
    ),
)]
#[get("/{org_id}/folders/{folder_id}")]
pub async fn get_folder(path: web::Path<(String, String)>) -> impl Responder {
    let (org_id, folder_id) = path.into_inner();
    match folders::get_folder(&org_id, &folder_id).await {
        Ok(folder) => {
            let body: CreateFolderResponseBody = folder.into();
            HttpResponse::Ok().json(body)
        }
        Err(err) => err.into(),
    }
}

/// DeleteFolder
#[utoipa::path(
    context_path = "/api",
    tag = "Folders",
    operation_id = "DeleteFolder",
    security(
        ("Authorization" = [])
    ),
    params(
        ("org_id" = String, Path, description = "Organization name"),
        ("folder_id" = String, Path, description = "Folder ID"),
    ),
    responses(
        (status = StatusCode::OK, description = "Success", body = HttpResponse),
        (status = StatusCode::NOT_FOUND, description = "NotFound", body = HttpResponse),
        (status = StatusCode::INTERNAL_SERVER_ERROR, description = "Error", body = HttpResponse),
    ),
)]
#[delete("/{org_id}/folders/{folder_id}")]
async fn delete_folder(path: web::Path<(String, String)>) -> impl Responder {
    let (org_id, folder_id) = path.into_inner();
    match folders::delete_folder(&org_id, &folder_id).await {
        Ok(()) => HttpResponse::Ok().body("Folder deleted"),
        Err(err) => err.into(),
    }
}
