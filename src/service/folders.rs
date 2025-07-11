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

use config::{
    ider,
    meta::{
        alerts::alert::ListAlertsParams,
        dashboards::{ListDashboardsParams, reports::ListReportsParams},
        folder::{DEFAULT_FOLDER, Folder, FolderType},
    },
};
use infra::{
    db::{ORM_CLIENT, connect_to_orm},
    table,
};
#[cfg(feature = "enterprise")]
use o2_openfga::meta::mapping::OFGA_MODELS;

use crate::common::{
    meta::authz::Authz,
    utils::auth::{remove_ownership, set_ownership},
};

/// Errors that can occur when interacting with folders.
#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    /// An error that occurs while interacting with the database through the
    /// [infra] crate.
    #[error("InfraError# Internal error")]
    InfraError(#[from] infra::errors::Error),

    /// An error that occurs while interacting with reports through the [infra::table::reports]
    /// module.
    #[error("ReportsError# Internal error")]
    TableReportsError(#[from] table::reports::Error),

    /// An error that occurs when trying to set a folder name to the empty string.
    #[error("Folder name cannot be empty")]
    MissingName,

    /// An error that occurs when trying to create a folder with a name
    /// that already exists in the same organization.
    #[error("Folder with this name already exists in this organization")]
    FolderNameAlreadyExists,

    /// An error that occurs when trying to update the special "default" folder.
    #[error("Can't update default folder")]
    UpdateDefaultFolder,

    /// An error that occurs when trying to delete a folder that contains dashboards.
    #[error("Folder contains dashboards. Please move/delete dashboards from folder.")]
    DeleteWithDashboards,

    /// An error that occurs when trying to delete a folder that contains alerts.
    #[error("Folder contains alerts. Please move/delete alerts from folder.")]
    DeleteWithAlerts,

    /// An error that occurs when trying to delete a folder that contains reports.
    #[error("Folder contains reports. Please move/delete reports from folder.")]
    DeleteWithReports,

    /// An error that occurs when trying to delete a folder that cannot be found.
    #[error("Folder not found")]
    NotFound,

    /// An error occured trying to get the list of permitted folders in
    /// enterprise mode because no user_id was provided.
    #[error("user_id required to get permitted folders in enterprise mode")]
    PermittedFoldersMissingUser,

    /// An error occured trying to get the list of permitted folders in
    /// enterprise mode using the validator.
    #[error("PermittedFoldersValidator# {0}")]
    PermittedFoldersValidator(String),
}

#[tracing::instrument(skip(folder))]
pub async fn save_folder(
    org_id: &str,
    mut folder: Folder,
    folder_type: FolderType,
    is_internal: bool,
) -> Result<Folder, FolderError> {
    folder.name = folder.name.trim().to_string();
    if folder.name.is_empty() {
        return Err(FolderError::MissingName);
    }

    if !is_internal && folder.folder_id == DEFAULT_FOLDER {
        return Err(FolderError::UpdateDefaultFolder);
    }

    if folder.folder_id != DEFAULT_FOLDER {
        folder.folder_id = ider::generate();
    }

    // Check if there is already a folder with the same name in the organization
    if get_folder_by_name(org_id, &folder.name, folder_type)
        .await
        .is_ok()
    {
        return Err(FolderError::FolderNameAlreadyExists);
    }

    let (_id, folder) = table::folders::put(org_id, None, folder, folder_type).await?;
    let folder_type_ofga = match folder_type {
        FolderType::Dashboards => "folders",
        FolderType::Alerts => "alert_folders",
        FolderType::Reports => "report_folders",
    };
    set_ownership(org_id, folder_type_ofga, Authz::new(&folder.folder_id)).await;

    #[cfg(feature = "enterprise")]
    if o2_enterprise::enterprise::common::config::get_config()
        .super_cluster
        .enabled
    {
        let _ = o2_enterprise::enterprise::super_cluster::queue::folders_create(
            org_id,
            _id,
            &folder.folder_id,
            folder_type,
            &folder.name,
            Some(folder.description.as_str()).filter(|d| !d.is_empty()),
        )
        .await;
    }

    Ok(folder)
}

#[tracing::instrument(skip(folder))]
pub async fn update_folder(
    org_id: &str,
    folder_id: &str,
    folder_type: FolderType,
    mut folder: Folder,
) -> Result<Folder, FolderError> {
    if folder_id.eq(DEFAULT_FOLDER) {
        return Err(FolderError::UpdateDefaultFolder);
    }

    folder.folder_id = folder_id.to_string();
    if let Ok(existing_folder) = get_folder_by_name(org_id, &folder.name, folder_type).await
        && existing_folder.folder_id != folder_id
    {
        return Err(FolderError::FolderNameAlreadyExists);
    }
    let (_, folder) = table::folders::put(org_id, None, folder, folder_type).await?;

    #[cfg(feature = "enterprise")]
    if o2_enterprise::enterprise::common::config::get_config()
        .super_cluster
        .enabled
    {
        let _ = o2_enterprise::enterprise::super_cluster::queue::folders_update(
            org_id,
            folder_id,
            folder_type,
            &folder.name,
            Some(folder.description.as_str()).filter(|d| !d.is_empty()),
        )
        .await;
    }

    Ok(folder)
}

#[tracing::instrument()]
pub async fn list_folders(
    org_id: &str,
    user_id: Option<&str>,
    folder_type: FolderType,
) -> Result<Vec<Folder>, FolderError> {
    let permitted_folders = permitted_folders(org_id, user_id, folder_type).await?;
    let folders = table::folders::list_folders(org_id, folder_type).await?;
    #[cfg(feature = "enterprise")]
    let folder_ofga_model = match folder_type {
        FolderType::Dashboards => OFGA_MODELS.get("folders").unwrap().key,
        FolderType::Alerts => OFGA_MODELS.get("alert_folders").unwrap().key,
        FolderType::Reports => OFGA_MODELS.get("report_folders").unwrap().key,
    };
    #[cfg(not(feature = "enterprise"))]
    let folder_ofga_model = "";

    let filtered = match permitted_folders {
        Some(permitted_folders) => {
            if permitted_folders.contains(&format!("{folder_ofga_model}:_all_{org_id}")) {
                folders
            } else {
                folders
                    .into_iter()
                    .filter(|folder_loc| {
                        permitted_folders
                            .contains(&format!("{folder_ofga_model}:{}", folder_loc.folder_id))
                    })
                    .collect::<Vec<_>>()
            }
        }
        None => folders,
    };
    Ok(filtered)
}

#[tracing::instrument()]
pub async fn get_folder(
    org_id: &str,
    folder_id: &str,
    folder_type: FolderType,
) -> Result<Folder, FolderError> {
    table::folders::get(org_id, folder_id, folder_type)
        .await?
        .ok_or(FolderError::NotFound)
}

#[tracing::instrument()]
pub async fn get_folder_by_name(
    org_id: &str,
    folder_name: &str,
    folder_type: FolderType,
) -> Result<Folder, FolderError> {
    table::folders::get_by_name(org_id, folder_name, folder_type)
        .await?
        .ok_or(FolderError::NotFound)
}

#[tracing::instrument()]
pub async fn delete_folder(
    org_id: &str,
    folder_id: &str,
    folder_type: FolderType,
) -> Result<(), FolderError> {
    let client = ORM_CLIENT.get_or_init(connect_to_orm).await;
    match folder_type {
        FolderType::Dashboards => {
            let params = ListDashboardsParams::new(org_id).with_folder_id(folder_id);
            let dashboards = table::dashboards::list(params).await?;
            if !dashboards.is_empty() {
                return Err(FolderError::DeleteWithDashboards);
            }
        }
        FolderType::Alerts => {
            let params = ListAlertsParams::new(org_id).in_folder(folder_id);
            let alerts = table::alerts::list(client, params).await?;
            if !alerts.is_empty() {
                return Err(FolderError::DeleteWithAlerts);
            }
        }
        FolderType::Reports => {
            let params = ListReportsParams::new(org_id).in_folder(folder_id);
            let reports = table::reports::list_reports(client, &params).await?;
            if !reports.is_empty() {
                return Err(FolderError::DeleteWithReports);
            }
        }
    };

    if !table::folders::exists(org_id, folder_id, folder_type).await? {
        return Err(FolderError::NotFound);
    }

    table::folders::delete(org_id, folder_id, folder_type).await?;
    let folder_type_ofga = match folder_type {
        FolderType::Dashboards => "folders",
        FolderType::Alerts => "alert_folders",
        FolderType::Reports => "report_folders",
    };
    remove_ownership(org_id, folder_type_ofga, Authz::new(folder_id)).await;

    #[cfg(feature = "enterprise")]
    if o2_enterprise::enterprise::common::config::get_config()
        .super_cluster
        .enabled
    {
        let _ = o2_enterprise::enterprise::super_cluster::queue::folders_delete(
            org_id,
            folder_id,
            folder_type,
        )
        .await;
    }

    Ok(())
}

#[cfg(not(feature = "enterprise"))]
async fn permitted_folders(
    _org_id: &str,
    _user_id: Option<&str>,
    _folder_type: FolderType,
) -> Result<Option<Vec<String>>, FolderError> {
    Ok(None)
}

#[cfg(feature = "enterprise")]
async fn permitted_folders(
    org_id: &str,
    user_id: Option<&str>,
    folder_type: FolderType,
) -> Result<Option<Vec<String>>, FolderError> {
    let (folder_ofga_model, child_ofga_model) = match folder_type {
        FolderType::Dashboards => (
            OFGA_MODELS.get("folders").unwrap().key,
            OFGA_MODELS.get("dashboards").unwrap().key,
        ),
        FolderType::Alerts => (
            OFGA_MODELS.get("alert_folders").unwrap().key,
            OFGA_MODELS.get("alerts").unwrap().key,
        ),
        FolderType::Reports => (
            OFGA_MODELS.get("report_folders").unwrap().key,
            OFGA_MODELS.get("reports").unwrap().key,
        ),
    };

    let Some(user_id) = user_id else {
        return Err(FolderError::PermittedFoldersMissingUser);
    };

    // Get the list of folders that the user has `GET` permission on.
    let mut folder_list = crate::handler::http::auth::validator::list_objects_for_user(
        org_id,
        user_id,
        "GET",
        folder_ofga_model,
    )
    .await
    .map_err(|err| FolderError::PermittedFoldersValidator(err.to_string()))?;

    // In some cases, there might not be direct `GET` permission on the folder.
    // So, we need to check if the user has `GET` permission on any of the dashboards
    // inside the folder.

    let permitted_dashboards = crate::handler::http::auth::validator::list_objects_for_user(
        org_id,
        user_id,
        "GET_INDIVIDUAL_FROM_ROLE",
        child_ofga_model,
    )
    .await
    .map_err(|err| FolderError::PermittedFoldersValidator(err.to_string()))?;

    log::debug!("permitted_dashboards: {permitted_dashboards:?}");
    if let Some(permitted_dashboards) = permitted_dashboards {
        let mut folder_list_with_roles = vec![];
        for dashboard in permitted_dashboards {
            let Some((_, folder_id)) = dashboard.split_once(":") else {
                continue;
            };
            // The folder_id is of the format `{folder_id}/{dashboard_id}`.
            // So, we need to extract the folder_id from the dashboard string.
            let Some((folder_id, _)) = folder_id.split_once("/") else {
                continue;
            };
            log::info!("folder_id: {folder_id:?}");
            folder_list_with_roles.push(format!("{folder_ofga_model}:{folder_id}"));
        }
        if let Some(folder_list) = folder_list.as_mut() {
            folder_list.extend(folder_list_with_roles);
        } else {
            folder_list = Some(folder_list_with_roles);
        }
    }
    log::info!("folder_list: {folder_list:?}");

    Ok(folder_list)
}
