use fetchx_remote_protocol::RemoteCommand;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Admin,
    Operator,
    ReadOnly,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::ReadOnly => "read-only",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "read-only" => Some(Self::ReadOnly),
            _ => None,
        }
    }

    const fn allows(self, permission: Permission) -> bool {
        match self {
            Self::Admin => true,
            Self::Operator => matches!(
                permission,
                Permission::ServerRead
                    | Permission::DownloadsRead
                    | Permission::DownloadsCreate
                    | Permission::DownloadsControl
                    | Permission::DownloadsPriority
                    | Permission::EventsRead
            ),
            Self::ReadOnly => {
                matches!(
                    permission,
                    Permission::ServerRead | Permission::DownloadsRead
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    ServerRead,
    DownloadsRead,
    DownloadsCreate,
    DownloadsControl,
    DownloadsRemove,
    DownloadsPriority,
    DownloadsManage,
    EventsRead,
}

pub struct AuthenticatedPrincipal {
    identity: CredentialIdentity,
    role: Role,
}

impl AuthenticatedPrincipal {
    pub(crate) const fn primary(role: Role) -> Self {
        Self {
            identity: CredentialIdentity::Primary,
            role,
        }
    }

    pub const fn identity(&self) -> &'static str {
        match self.identity {
            CredentialIdentity::Primary => "primary",
        }
    }

    pub const fn role(&self) -> Role {
        self.role
    }

    pub const fn allows(&self, permission: Permission) -> bool {
        self.role.allows(permission)
    }
}

enum CredentialIdentity {
    Primary,
}

pub(crate) const fn required_permission(command: &RemoteCommand) -> Permission {
    match command {
        RemoteCommand::Discover => Permission::ServerRead,
        RemoteCommand::DownloadGet(_) | RemoteCommand::DownloadList => Permission::DownloadsRead,
        RemoteCommand::DownloadCreate(_) => Permission::DownloadsCreate,
        RemoteCommand::DownloadPause(_)
        | RemoteCommand::DownloadResume(_)
        | RemoteCommand::DownloadCancel(_)
        | RemoteCommand::DownloadRetry(_) => Permission::DownloadsControl,
        RemoteCommand::DownloadRemove(_) => Permission::DownloadsRemove,
        RemoteCommand::DownloadSetPriority { .. } => Permission::DownloadsPriority,
        RemoteCommand::DownloadSetEngine { .. } | RemoteCommand::DownloadSetSpeedLimit { .. } => {
            Permission::DownloadsManage
        }
    }
}

#[cfg(test)]
mod tests {
    use fetchx_models::{DownloadId, DownloadPriority, EngineKind};
    use fetchx_remote_protocol::{DownloadCreateParams, RemoteCommand};

    use super::{AuthenticatedPrincipal, Permission, Role, required_permission};

    fn commands() -> Vec<(RemoteCommand, Permission)> {
        let id = DownloadId::new();
        vec![
            (RemoteCommand::Discover, Permission::ServerRead),
            (
                RemoteCommand::DownloadCreate(DownloadCreateParams {
                    url: "https://example.invalid/file".to_owned(),
                    engine: None,
                    priority: None,
                }),
                Permission::DownloadsCreate,
            ),
            (RemoteCommand::DownloadGet(id), Permission::DownloadsRead),
            (RemoteCommand::DownloadList, Permission::DownloadsRead),
            (
                RemoteCommand::DownloadPause(id),
                Permission::DownloadsControl,
            ),
            (
                RemoteCommand::DownloadResume(id),
                Permission::DownloadsControl,
            ),
            (
                RemoteCommand::DownloadCancel(id),
                Permission::DownloadsControl,
            ),
            (
                RemoteCommand::DownloadRetry(id),
                Permission::DownloadsControl,
            ),
            (
                RemoteCommand::DownloadRemove(id),
                Permission::DownloadsRemove,
            ),
            (
                RemoteCommand::DownloadSetPriority {
                    id,
                    priority: DownloadPriority::Normal,
                },
                Permission::DownloadsPriority,
            ),
            (
                RemoteCommand::DownloadSetEngine {
                    id,
                    engine: EngineKind::DirectHttp,
                },
                Permission::DownloadsManage,
            ),
            (
                RemoteCommand::DownloadSetSpeedLimit {
                    id,
                    bytes_per_second: Some(1024),
                },
                Permission::DownloadsManage,
            ),
        ]
    }

    #[test]
    fn every_protocol_command_has_an_explicit_permission() {
        assert_eq!(commands().len(), 12);
        for (command, expected) in commands() {
            assert_eq!(required_permission(&command), expected);
        }
    }

    #[test]
    fn role_permissions_follow_the_least_privilege_matrix() {
        let cases = [
            (Role::Admin, true, true, true, true),
            (Role::Operator, true, true, true, false),
            (Role::ReadOnly, true, false, false, false),
        ];
        for (role, can_read, can_create, can_events, can_remove) in cases {
            let principal = AuthenticatedPrincipal::primary(role);
            assert_eq!(principal.allows(Permission::DownloadsRead), can_read);
            assert_eq!(principal.allows(Permission::DownloadsCreate), can_create);
            assert_eq!(principal.allows(Permission::EventsRead), can_events);
            assert_eq!(principal.allows(Permission::DownloadsRemove), can_remove);
            assert_eq!(principal.identity(), "primary");
            assert_eq!(principal.role(), role);
        }
    }

    #[test]
    fn role_names_are_stable_and_reject_unknown_values() {
        for role in [Role::Admin, Role::Operator, Role::ReadOnly] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("owner"), None);
    }
}
