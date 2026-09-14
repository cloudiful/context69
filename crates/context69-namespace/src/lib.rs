mod ports;
mod service;
mod types;

pub use context69_contracts_namespace::{GroupKind, MembershipRole, Visibility};
pub use ports::{NamespaceRepository, Page, PageRequest, PageSort};
pub use service::NamespaceService;
pub use types::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, PersonalGroupRecord, UpdateGroupInput, UpsertMembershipInput,
};
