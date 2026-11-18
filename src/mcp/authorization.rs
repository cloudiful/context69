use crate::{api::RequestAuth, contracts::PersonalAccessTokenScope};
use rmcp::{
    ErrorData, ServerHandler,
    model::*,
    service::{RequestContext, RoleServer},
};

use super::{resource_scope, tool_scope};

#[cfg(test)]
mod tests;

pub(super) struct Authorized<S>(S);

impl<S> Authorized<S> {
    pub(super) fn new(inner: S) -> Self {
        Self(inner)
    }
}

fn allowed(context: &RequestContext<RoleServer>, scope: Option<PersonalAccessTokenScope>) -> bool {
    let Some(parts) = context.extensions.get::<axum::http::request::Parts>() else {
        return true;
    };
    parts
        .extensions
        .get::<RequestAuth>()
        .is_some_and(|auth| auth.permits_scope(scope))
}

fn denied() -> ErrorData {
    ErrorData::invalid_request("personal access token missing required scope", None)
}

impl<S: ServerHandler> ServerHandler for Authorized<S> {
    fn get_info(&self) -> ServerConfig {
        self.0.get_info()
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.0.get_tool(name)
    }
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        self.0.supported_protocol_versions()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = self.0.list_tools(request, context.clone()).await?;
        result
            .tools
            .retain(|tool| allowed(&context, tool_scope(tool.name.as_ref())));
        Ok(result)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !allowed(&context, tool_scope(request.name.as_ref())) {
            return Err(denied());
        }
        self.0.call_tool(request, context).await
    }
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut result = self.0.list_resources(request, context.clone()).await?;
        let original_count = result.resources.len();
        result
            .resources
            .retain(|resource| allowed(&context, resource_scope(&resource.uri)));
        if result.resources.len() != original_count
            || !allowed(&context, Some(PersonalAccessTokenScope::Sources))
        {
            result.next_cursor = None;
        }
        Ok(result)
    }
    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.0.list_resource_templates(request, context).await
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if !allowed(&context, resource_scope(&request.uri)) {
            return Err(denied());
        }
        self.0.read_resource(request, context).await
    }
}
