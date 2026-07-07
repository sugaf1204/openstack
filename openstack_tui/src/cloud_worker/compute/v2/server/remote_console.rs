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

use derive_builder::Builder;
use eyre::{Report, Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::cloud_worker::common::CloudWorkerError;
use crate::cloud_worker::types::{ApiRequest, ExecuteApiRequest};

use openstack_sdk::api::QueryAsync;
use openstack_sdk::api::compute::v2::server::remote_console::create_26::{
    Protocol, RemoteConsoleBuilder, RequestBuilder, Type,
};
use openstack_sdk::AsyncOpenStack;

#[derive(Builder, Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[builder(setter(strip_option))]
pub struct ComputeServerCreateRemoteConsole {
    pub server_id: String,
    #[builder(default)]
    pub name: Option<String>,
}

impl fmt::Display for ComputeServerCreateRemoteConsole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "name/id: {}",
            self.name.clone().unwrap_or_else(|| self.server_id.clone())
        ));
        parts.push(String::from("protocol: vnc"));
        parts.push(String::from("type: novnc"));
        write!(f, "{}", parts.join(","))
    }
}

impl TryFrom<&ComputeServerCreateRemoteConsole> for RequestBuilder<'_> {
    type Error = Report;

    fn try_from(value: &ComputeServerCreateRemoteConsole) -> Result<Self, Self::Error> {
        let mut remote_console_builder = RemoteConsoleBuilder::default();
        remote_console_builder
            .protocol(Protocol::Vnc)
            ._type(Type::Novnc);

        let mut ep_builder = Self::default();
        ep_builder.server_id(value.server_id.clone());
        ep_builder.remote_console(
            remote_console_builder
                .build()
                .wrap_err("cannot prepare request element `RemoteConsole`")?,
        );

        Ok(ep_builder)
    }
}

impl ExecuteApiRequest for ComputeServerCreateRemoteConsole {
    async fn execute_request(
        &self,
        session: &mut AsyncOpenStack,
        request: &ApiRequest,
        app_tx: &UnboundedSender<Action>,
    ) -> Result<(), CloudWorkerError> {
        let ep = TryInto::<RequestBuilder>::try_into(self)?
            .build()
            .wrap_err("Cannot prepare request")?;
        let data: serde_json::Value = ep.query_async(session).await?;
        app_tx.send(Action::ApiResponseData {
            request: request.clone(),
            data,
        })?;
        Ok(())
    }
}
