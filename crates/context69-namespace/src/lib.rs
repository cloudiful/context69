mod ports;
mod service;
mod types;

pub use context69_contracts::{GroupKind, MembershipRole, Visibility};
pub use ports::NamespaceRepository;
pub use service::NamespaceService;
pub use types::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, PersonalGroupRecord, UpdateGroupInput, UpsertMembershipInput,
};
