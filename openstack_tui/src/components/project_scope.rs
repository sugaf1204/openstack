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

use openstack_sdk::{
    auth::authtoken::AuthTokenScope,
    types::identity::v3::{Domain, Project},
};

pub(crate) fn project_scope(
    id: Option<String>,
    name: Option<String>,
    domain_id: Option<String>,
    domain_name: Option<String>,
) -> AuthTokenScope {
    let use_id = id.is_some();
    let domain = if use_id {
        None
    } else {
        match (domain_id, domain_name) {
            (None, None) => None,
            (id, name) => Some(Domain { id, name }),
        }
    };

    AuthTokenScope::Project(Project {
        id,
        name: if use_id { None } else { name },
        domain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_scope_prefers_id() {
        let scope = project_scope(
            Some("project-id".into()),
            Some("Project Name".into()),
            Some("domain-id".into()),
            Some("Domain Name".into()),
        );

        let AuthTokenScope::Project(project) = scope else {
            panic!("expected project scope");
        };

        assert_eq!(project.id.as_deref(), Some("project-id"));
        assert_eq!(project.name, None);
        assert_eq!(project.domain, None);
    }

    #[test]
    fn project_scope_keeps_name_domain_when_id_missing() {
        let scope = project_scope(
            None,
            Some("Project Name".into()),
            Some("domain-id".into()),
            None,
        );

        let AuthTokenScope::Project(project) = scope else {
            panic!("expected project scope");
        };

        assert_eq!(project.id, None);
        assert_eq!(project.name.as_deref(), Some("Project Name"));
        assert_eq!(
            project.domain.as_ref().and_then(|domain| domain.id.as_deref()),
            Some("domain-id")
        );
        assert_eq!(
            project
                .domain
                .as_ref()
                .and_then(|domain| domain.name.as_deref()),
            None
        );
    }
}
