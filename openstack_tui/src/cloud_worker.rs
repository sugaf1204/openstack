// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0
//! Cloud worker module.
//!
//! Handle communication with the cloud including connection, re-connection (when auth expires) and
//! all the API requests.

use async_trait::async_trait;
use chrono::TimeDelta;
use eyre::{Report, Result, eyre};
use openstack_sdk::{
    AsyncOpenStack,
    auth::authtoken::AuthTokenScope,
    auth::AuthState,
    auth::auth_helper::{AuthHelper, AuthHelperError},
    config::{CloudConfig, ConfigFile},
    types::identity::v3::AuthResponse,
};
use secrecy::SecretString;
use std::path::PathBuf;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tracing::{debug, instrument, trace};

use crate::action::Action;

pub mod block_storage;
mod common;
pub mod compute;
pub mod dns;
pub mod identity;
pub mod image;
pub mod load_balancer;
pub mod network;
pub mod types;

pub use crate::cloud_worker::common::CloudWorkerError;

use crate::cloud_worker::types::*;
use crate::error::TuiError;

/// Cloud worker struct
pub(crate) struct Cloud {
    cloud_configs: ConfigFile,
    pub(crate) cloud: Option<AsyncOpenStack>,
    cloud_name: Option<String>,
    auth_helper: TuiAuthHelper,
}

#[derive(Debug)]
pub enum AuthAction {
    Data(String),
    Secret(SecretString),
    Cancel,
}

impl Cloud {
    pub fn new(
        client_config_config_file: Option<PathBuf>,
        client_secure_config_file: Option<PathBuf>,
        auth_helper_control_tx: mpsc::Sender<oneshot::Sender<AuthAction>>,
    ) -> Result<Self, TuiError> {
        let cfg = ConfigFile::new_with_user_specified_configs(
            client_config_config_file.as_deref(),
            client_secure_config_file.as_deref(),
        )?;

        Ok(Self {
            cloud_configs: cfg,
            cloud: None,
            cloud_name: None,
            auth_helper: TuiAuthHelper::new(auth_helper_control_tx),
        })
    }

    async fn connect_profile(
        &self,
        profile: &CloudConfig,
        renew_auth: bool,
    ) -> Result<AsyncOpenStack> {
        let session = AsyncOpenStack::new_with_authentication_helper(
            profile,
            self.auth_helper.clone(),
            renew_auth,
        )
        .await?;

        discover_tui_service_endpoints(&session).await?;

        Ok(session)
    }

    pub async fn connect_to_cloud(&mut self, cloud: String) -> Result<()> {
        debug!("Connecting to cloud {}", cloud);
        let profile = self
            .cloud_configs
            .get_cloud_config(cloud.clone())?
            .ok_or_else(|| eyre!("Cloud `{}` is not present in configuration files", cloud))?;
        let session = self.connect_profile(&profile, false).await?;

        self.cloud = Some(session);
        self.cloud_name = Some(cloud);

        Ok(())
    }

    pub async fn switch_auth_scope(
        &mut self,
        scope: &AuthTokenScope,
    ) -> Result<Option<AuthResponse>, Report> {
        if self.cloud.is_none() {
            return Err(eyre!("Cannot change scope without being connected first"));
        }
        let cloud_name = self
            .cloud_name
            .clone()
            .ok_or_else(|| eyre!("Cannot change scope without a selected cloud"))?;
        let current_region = self.cloud.as_ref().and_then(AsyncOpenStack::get_region_name);
        let profile = self
            .cloud_configs
            .get_cloud_config(cloud_name.clone())?
            .ok_or_else(|| eyre!("Cloud `{}` is not present in configuration files", cloud_name))?;
        let scoped_profile = profile_scoped_to(profile, scope);

        debug!("Switching connection scope to {:?}", scope);
        let session = self.connect_profile(&scoped_profile, true).await?;
        if let Some(region) = current_region {
            session.set_region_name(region)?;
            discover_tui_service_endpoints(&session).await?;
        }
        debug!("Authed as {:?}", session.get_auth_info());

        let auth_info = session.get_auth_info();
        self.cloud = Some(session);
        Ok(auth_info)
    }

    pub async fn run(
        &mut self,
        app_tx: UnboundedSender<Action>,
        action_rx: &mut UnboundedReceiver<Action>,
    ) -> Result<(), TuiError> {
        self.auth_helper.set_action_tx(app_tx.clone())?;
        while let Some(action) = action_rx.recv().await {
            debug!("Got action {:?}", action);
            match action {
                ref ac @ Action::ConnectToCloud(ref cloud) => {
                    match self.connect_to_cloud(cloud.clone()).await {
                        Ok(()) => {
                            if let Some(cloud) = &self.cloud
                                && let Some(auth_info) = cloud.get_auth_info()
                            {
                                app_tx.send(Action::ConnectedToCloud(Box::new(auth_info.token)))?;
                            }
                        }
                        Err(err) => app_tx.send(Action::Error {
                            msg: format!("Failed to connect to the cloud: {err:?}"),
                            action: Some(Box::new(ac.clone())),
                        })?,
                    }
                }
                Action::ListClouds => {
                    app_tx.send(Action::Clouds(self.cloud_configs.get_available_clouds()))?;
                }
                Action::CloudChangeScope(ref scope) => match self.switch_auth_scope(scope).await {
                    Ok(auth_response) => {
                        if let Some(auth_info) = auth_response {
                            app_tx.send(Action::ConnectedToCloud(Box::new(auth_info.token)))?;
                        }
                    }
                    Err(err) => app_tx.send(Action::Error {
                        msg: format!("Cannot switch session scope: {err:?}"),
                        action: Some(Box::new(action.clone())),
                    })?,
                },
                Action::SwitchToRegion(ref region) => match self.cloud {
                    Some(ref mut session) => {
                        debug!("Switching region to {}", region);
                        if let Err(err) = session.set_region_name(region.clone()) {
                            app_tx.send(Action::Error {
                                msg: format!("Cannot switch region: {err:?}"),
                                action: Some(Box::new(action.clone())),
                            })?;
                        } else {
                            session
                                .discover_service_endpoint(
                                    &openstack_sdk::types::ServiceType::Compute,
                                )
                                .await?;
                            session
                                .discover_service_endpoint(
                                    &openstack_sdk::types::ServiceType::BlockStorage,
                                )
                                .await?;
                            session
                                .discover_service_endpoint(
                                    &openstack_sdk::types::ServiceType::Image,
                                )
                                .await?;
                            session
                                .discover_service_endpoint(
                                    &openstack_sdk::types::ServiceType::Network,
                                )
                                .await?;
                            if let Some(auth_info) = session.get_auth_info() {
                                app_tx.send(Action::ConnectedToCloud(Box::new(auth_info.token)))?;
                            }
                        }
                    }
                    _ => app_tx.send(Action::Error {
                        msg: String::from("Cannot switch region without being connected first"),
                        action: Some(Box::new(action.clone())),
                    })?,
                },
                Action::ListRegions => {
                    if let Some(ref session) = self.cloud
                        && let Some(regions) = session.get_available_regions()
                    {
                        app_tx.send(Action::Regions(regions))?;
                    }
                }
                ref ac @ Action::PerformApiRequest(ref request) => {
                    if let Some(ref mut conn) = self.cloud {
                        // Check if reauth is necessary
                        match &conn.get_auth_state(Some(TimeDelta::seconds(10))) {
                            Some(AuthState::Expired) | Some(AuthState::AboutToExpire) => {
                                conn.authorize(None, false, true).await?;
                            }
                            _ => {}
                        }

                        request
                            .execute_request(conn, request, &app_tx)
                            .await
                            .or_else(|err| {
                                app_tx.send(Action::Error {
                                    msg: format!("Error performing API request\n\n{err:?}"),
                                    action: Some(Box::new(ac.clone())),
                                })
                            })?;
                    }
                }
                _ => {}
            };
        }
        Ok(())
    }
}

async fn discover_tui_service_endpoints(session: &AsyncOpenStack) -> Result<()> {
    for service_type in [
        openstack_sdk::types::ServiceType::Compute,
        openstack_sdk::types::ServiceType::BlockStorage,
        openstack_sdk::types::ServiceType::Dns,
        openstack_sdk::types::ServiceType::Image,
        openstack_sdk::types::ServiceType::LoadBalancer,
        openstack_sdk::types::ServiceType::Network,
    ] {
        session.discover_service_endpoint(&service_type).await?;
    }
    Ok(())
}

fn profile_scoped_to(mut profile: CloudConfig, scope: &AuthTokenScope) -> CloudConfig {
    let auth = profile.auth.get_or_insert_with(Default::default);
    auth.project_id = None;
    auth.project_name = None;
    auth.project_domain_id = None;
    auth.project_domain_name = None;
    auth.domain_id = None;
    auth.domain_name = None;
    auth.system_scope = None;

    match scope {
        AuthTokenScope::Project(project) => {
            auth.project_id.clone_from(&project.id);
            if project.id.is_none() {
                auth.project_name.clone_from(&project.name);
                if let Some(domain) = &project.domain {
                    auth.project_domain_id.clone_from(&domain.id);
                    auth.project_domain_name.clone_from(&domain.name);
                }
            }
        }
        AuthTokenScope::Domain(domain) => {
            auth.domain_id.clone_from(&domain.id);
            auth.domain_name.clone_from(&domain.name);
        }
        AuthTokenScope::System(system) => {
            if system.all == Some(true) {
                auth.system_scope = Some(String::from("all"));
            }
        }
        AuthTokenScope::Unscoped => {}
    }

    profile.auth_cache = Some(false);
    profile
}

#[derive(Clone)]
struct TuiAuthHelper {
    app_tx: Option<UnboundedSender<Action>>,
    auth_helper_control_tx: mpsc::Sender<oneshot::Sender<AuthAction>>,
}

impl TuiAuthHelper {
    pub fn new(auth_helper_control_tx: mpsc::Sender<oneshot::Sender<AuthAction>>) -> Self {
        Self {
            app_tx: None,
            auth_helper_control_tx,
        }
    }

    pub fn set_action_tx(&mut self, app_tx: UnboundedSender<Action>) -> Result<(), TuiError> {
        self.app_tx = Some(app_tx);
        Ok(())
    }

    #[instrument(skip(self))]
    async fn initiate(
        &self,
        prompt: String,
        connection_name: Option<String>,
        is_sensitive: bool,
    ) -> Result<oneshot::Receiver<AuthAction>, TuiError> {
        let (sender, receiver) = oneshot::channel();
        self.auth_helper_control_tx.send(sender).await?;
        if let Some(app_tx) = &self.app_tx {
            trace!("Sending request to the app");
            app_tx.send(Action::AuthDataRequired {
                prompt,
                connection_name,
                is_sensitive,
            })?;
        } else {
            return Err(eyre!(
                "Channel between cloud worker and application is missing".to_string(),
            )
            .into());
        }
        Ok(receiver)
    }
}

#[async_trait]
impl AuthHelper for TuiAuthHelper {
    fn clone_box(&self) -> Box<dyn AuthHelper> {
        Box::new(self.clone())
    }

    #[instrument(skip(self))]
    async fn get(
        &self,
        prompt: String,
        connection_name: Option<String>,
    ) -> Result<String, AuthHelperError> {
        let receiver = self
            .initiate(prompt, connection_name, true)
            .await
            .map_err(|e| AuthHelperError::Other(e.to_string()))?;
        trace!("Waiting for the auth data to arrive from the UI");
        match receiver.await {
            Ok(AuthAction::Data(dt)) => {
                trace!("auth data received");
                return Ok(dt);
            }
            _ => {
                trace!("auth data request cancelled");
                return Err(AuthHelperError::Other(
                    "error receiving the requested data".to_string(),
                ));
            }
        }
    }

    #[instrument(skip(self))]
    async fn get_secret(
        &self,
        prompt: String,
        connection_name: Option<String>,
    ) -> Result<SecretString, AuthHelperError> {
        let receiver = self
            .initiate(prompt, connection_name, true)
            .await
            .map_err(|e| AuthHelperError::Other(e.to_string()))?;
        trace!("Waiting for the auth data to arrive from the UI");
        match receiver.await {
            Ok(AuthAction::Secret(dt)) => {
                trace!("auth data received");
                Ok(dt)
            }
            _ => {
                trace!("auth data request cancelled");
                return Err(AuthHelperError::Other(
                    "error receiving the requested data".to_string(),
                ));
            }
        }
    }
}

impl ExecuteApiRequest for ApiRequest {
    async fn execute_request(
        &self,
        session: &mut AsyncOpenStack,
        request: &ApiRequest,
        app_tx: &UnboundedSender<Action>,
    ) -> Result<(), CloudWorkerError> {
        match self {
            ApiRequest::BlockStorage(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::Compute(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::Dns(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::Identity(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::Image(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::LoadBalancer(data) => data.execute_request(session, request, app_tx).await,
            ApiRequest::Network(data) => data.execute_request(session, request, app_tx).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openstack_sdk::config::Auth;
    use openstack_sdk::types::identity::v3::{Domain, Project};

    fn profile_with_existing_scope() -> CloudConfig {
        CloudConfig {
            auth: Some(Auth {
                project_id: Some(String::from("old-project-id")),
                project_name: Some(String::from("old-project-name")),
                project_domain_id: Some(String::from("old-domain-id")),
                project_domain_name: Some(String::from("old-domain-name")),
                domain_id: Some(String::from("old-domain-scope-id")),
                domain_name: Some(String::from("old-domain-scope-name")),
                system_scope: Some(String::from("all")),
                ..Default::default()
            }),
            auth_cache: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn profile_scoped_to_project_id_clears_other_scope_fields() {
        let profile = profile_scoped_to(
            profile_with_existing_scope(),
            &AuthTokenScope::Project(Project {
                id: Some(String::from("new-project-id")),
                name: Some(String::from("new-project-name")),
                domain: Some(Domain {
                    id: Some(String::from("new-domain-id")),
                    name: Some(String::from("new-domain-name")),
                }),
            }),
        );
        let auth = profile.auth.unwrap();

        assert_eq!(auth.project_id.as_deref(), Some("new-project-id"));
        assert_eq!(auth.project_name, None);
        assert_eq!(auth.project_domain_id, None);
        assert_eq!(auth.project_domain_name, None);
        assert_eq!(auth.domain_id, None);
        assert_eq!(auth.domain_name, None);
        assert_eq!(auth.system_scope, None);
        assert_eq!(profile.auth_cache, Some(false));
    }

    #[test]
    fn profile_scoped_to_project_name_keeps_domain() {
        let profile = profile_scoped_to(
            profile_with_existing_scope(),
            &AuthTokenScope::Project(Project {
                id: None,
                name: Some(String::from("new-project-name")),
                domain: Some(Domain {
                    id: Some(String::from("new-domain-id")),
                    name: None,
                }),
            }),
        );
        let auth = profile.auth.unwrap();

        assert_eq!(auth.project_id, None);
        assert_eq!(auth.project_name.as_deref(), Some("new-project-name"));
        assert_eq!(auth.project_domain_id.as_deref(), Some("new-domain-id"));
        assert_eq!(auth.project_domain_name, None);
        assert_eq!(profile.auth_cache, Some(false));
    }
}
