mod ports;
mod service;
mod types;

pub use context69_contracts::{GroupKind, MembershipRole, Visibility};
pub use ports::NamespaceRepository;
pub use service::NamespaceService;
pub use types::{
    AccessScope, CreateGroupInput, CreateProjectInput, GroupRecord, MoveProjectInput,
    NamespaceActor, NamespaceMemberRecord, PersonalGroupRecord, ProjectRecord, UpdateGroupInput,
    UpdateProjectInput, UpsertMembershipInput,
};
