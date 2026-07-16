//! Helper-module body emitters. Each expands to the body of a plugin helper
//! module (`host`/`bag`/`manifest`). The `plugin!` macro wraps each in a
//! `mod`. All `crate::jinn::plugin::*` paths resolve in the invoking plugin
//! crate, where `generate!` produced them.

#[macro_export]
macro_rules! __jinn_pdk_host_body {
    () => {
        use crate::jinn::plugin::host;
        use crate::jinn::plugin::types::{
            Command, CreateSessionReq, CreateSessionResp, EnqueueUserMessageCmd, FireAsyncHookCmd,
            LlmOneshotReq, LlmResp, PushChatEntryCmd, PushEntryKind, RequestError, SetChatInputCmd,
            SetChatInputEnabledCmd,
        };

        /// Outcome of `request_llm_oneshot` after desugaring the WIT result variant.
        #[derive(Debug)]
        pub enum LlmOutcome {
            Ok(LlmResp),
            Cancelled,
            Other(String),
        }

        /// Outcome of `create_session` after desugaring the WIT result variant.
        #[derive(Debug)]
        pub enum CreateSessionOutcome {
            Ok(CreateSessionResp),
            Cancelled,
            Other(String),
        }

        fn desugar_error(e: RequestError) -> String {
            match e {
                RequestError::Cancelled => String::from("cancelled"),
                RequestError::Other(s) => s,
            }
        }

        /// History-less one-shot LLM call. Inherits the session's provider+model.
        ///
        /// Awaits the host's future (suspends the component stack), then resumes with
        /// the response. Must be called from an async hook body.
        pub async fn request_llm_oneshot(req: LlmOneshotReq) -> LlmOutcome {
            match host::request_llm_oneshot(req).await {
                Ok(resp) => LlmOutcome::Ok(resp),
                Err(RequestError::Cancelled) => LlmOutcome::Cancelled,
                Err(e) => LlmOutcome::Other(desugar_error(e)),
            }
        }

        /// Create a child session under `parent-session-id`. Used by the judge plugin.
        ///
        /// Awaits the host's future (suspends the component stack), then resumes with
        /// the new session id. Must be called from an async hook body.
        pub async fn create_session(req: CreateSessionReq) -> CreateSessionOutcome {
            match host::create_session(req).await {
                Ok(resp) => CreateSessionOutcome::Ok(resp),
                Err(RequestError::Cancelled) => CreateSessionOutcome::Cancelled,
                Err(e) => CreateSessionOutcome::Other(desugar_error(e)),
            }
        }

        /// Cancel a named in-flight request (matched against its `task` field).
        pub fn cancel_task(name: &str) {
            host::cancel_task(name);
        }

        /// Fire-and-forget a typed domain command.
        pub fn emit(cmd: Command) {
            host::emit(&cmd);
        }

        /// Push a system / transient / error entry into a session's chat log.
        pub fn push_system_entry(session_id: &str, text: impl Into<String>) {
            emit(Command::PushChatEntry(PushChatEntryCmd {
                session_id: session_id.to_owned(),
                kind: PushEntryKind::System(text.into()),
            }));
        }

        pub fn push_transient_entry(session_id: &str, text: impl Into<String>) {
            emit(Command::PushChatEntry(PushChatEntryCmd {
                session_id: session_id.to_owned(),
                kind: PushEntryKind::Transient(text.into()),
            }));
        }

        pub fn push_error_entry(session_id: &str, text: impl Into<String>) {
            emit(Command::PushChatEntry(PushChatEntryCmd {
                session_id: session_id.to_owned(),
                kind: PushEntryKind::Error(text.into()),
            }));
        }

        /// Replace the chat-input draft for a session.
        pub fn set_chat_input(session_id: &str, text: &str) {
            emit(Command::SetChatInput(SetChatInputCmd {
                session_id: session_id.to_owned(),
                text: text.to_owned(),
            }));
        }

        /// Enable/disable the chat input for a session (e.g. while enriching).
        pub fn set_chat_input_enabled(session_id: &str, enabled: bool) {
            emit(Command::SetChatInputEnabled(SetChatInputEnabledCmd {
                session_id: session_id.to_owned(),
                enabled,
            }));
        }

        /// Enqueue a user-authored message into a session.
        pub fn enqueue_user_message(session_id: &str, text: &str) {
            emit(Command::EnqueueUserMessage(EnqueueUserMessageCmd {
                session_id: session_id.to_owned(),
                text: text.to_owned(),
            }));
        }

        /// Fire an async hook by name (runtime export lookup). Used by sync hooks to
        /// kick off async work via the host (no async import is callable from sync).
        pub fn fire_async_hook(session_id: &str, hook: &str, text: Option<String>) {
            emit(Command::FireAsyncHook(FireAsyncHookCmd {
                session_id: session_id.to_owned(),
                hook: hook.to_owned(),
                text,
            }));
        }
    };
}

#[macro_export]
macro_rules! __jinn_pdk_bag_body {
    () => {
        use serde::{de::DeserializeOwned, Serialize};

        /// Read and deserialize the plugin's own state bag.
        ///
        /// Returns `None` when the bag is empty, unset, or fails to deserialize (a
        /// bad decode is treated as an empty bag).
        #[must_use]
        pub fn get_plugin_data<T: DeserializeOwned + Default>() -> Option<T> {
            let bytes = crate::jinn::plugin::host::get_plugin_data()?;
            decode::<T>(&bytes)
        }

        /// Serialize and write the plugin's own state bag, replacing any prior value.
        pub fn set_plugin_data<T: Serialize>(value: &T) {
            crate::jinn::plugin::host::set_plugin_data(&encode(value));
        }

        /// Read a cross-instance shared value from the global-data bag.
        ///
        /// Returns `None` when the key is unset or fails to deserialize.
        /// Used for multi-instance coordination (e.g. judge aggregation).
        #[must_use]
        pub fn get_global_data<T: DeserializeOwned + Default>(key: &str) -> Option<T> {
            crate::jinn::plugin::host::get_global_data(key).and_then(|bytes| decode::<T>(&bytes))
        }

        /// Serialize and write a cross-instance shared value.
        pub fn set_global_data<T: Serialize>(key: &str, value: &T) {
            let bytes = encode(value);
            crate::jinn::plugin::host::set_global_data(key, Some(&bytes));
        }

        fn encode<T: Serialize>(value: &T) -> Vec<u8> {
            postcard::to_stdvec(value).expect("plugin-data serialization must not fail")
        }

        fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
            postcard::from_bytes(bytes).ok()
        }
    };
}

#[macro_export]
macro_rules! __jinn_pdk_manifest_body {
    () => {
        use crate::jinn::plugin::types::{
            BadgeSegment, Keybind as WitKeybind, Manifest as WitManifest, ToolDecl as WitToolDecl,
            ToolParam as WitToolParam, ToolScope as WitToolScope,
        };

        /// A plugin's declared metadata: keybinds + tools. Built via the builder API.
        #[derive(Debug, Default)]
        pub struct Manifest {
            pub keybinds: Vec<Keybind>,
            pub tools: Vec<Tool>,
            pub description: Option<String>,
        }

        impl Manifest {
            #[must_use]
            pub fn new() -> Self {
                Self::default()
            }

            pub fn with_description(mut self, description: impl Into<String>) -> Self {
                self.description = Some(description.into());
                self
            }

            pub fn with_keybind(mut self, kb: Keybind) -> Self {
                self.keybinds.push(kb);
                self
            }

            pub fn with_tool(mut self, tool: Tool) -> Self {
                self.tools.push(tool);
                self
            }

            /// Convert into the generated WIT manifest record for `get-manifest()`.
            #[must_use]
            pub fn into_wit(self) -> WitManifest {
                WitManifest {
                    description: self.description,
                    keybinds: self.keybinds.into_iter().map(Keybind::into_wit).collect(),
                    tools: self.tools.into_iter().map(Tool::into_wit).collect(),
                }
            }
        }

        /// One declared keybind. `action` names the async hook the host fires.
        #[derive(Debug)]
        pub struct Keybind {
            pub scope: String,
            pub keys: String,
            pub action: String,
            pub description: String,
        }

        impl Keybind {
            #[must_use]
            pub fn new(
                scope: impl Into<String>,
                keys: impl Into<String>,
                action: impl Into<String>,
            ) -> Self {
                Self {
                    scope: scope.into(),
                    keys: keys.into(),
                    action: action.into(),
                    description: String::new(),
                }
            }

            pub fn described_as(mut self, description: impl Into<String>) -> Self {
                self.description = description.into();
                self
            }

            #[must_use]
            pub fn into_wit(self) -> WitKeybind {
                WitKeybind {
                    scope: self.scope,
                    keys: self.keys,
                    action: self.action,
                    description: self.description,
                }
            }
        }

        /// One declared tool. Discovered by the host at runtime.
        #[derive(Debug)]
        pub struct Tool {
            pub name: String,
            pub description: String,
            pub parameters: Vec<ToolParam>,
            pub scope: ToolScope,
        }

        impl Tool {
            #[must_use]
            pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
                Self {
                    name: name.into(),
                    description: description.into(),
                    parameters: Vec::new(),
                    scope: ToolScope::Attached,
                }
            }

            pub fn global(mut self) -> Self {
                self.scope = ToolScope::Global;
                self
            }

            pub fn attached(mut self) -> Self {
                self.scope = ToolScope::Attached;
                self
            }

            pub fn with_param(mut self, param: ToolParam) -> Self {
                self.parameters.push(param);
                self
            }

            #[must_use]
            pub fn into_wit(self) -> WitToolDecl {
                WitToolDecl {
                    name: self.name,
                    description: self.description,
                    parameters: self
                        .parameters
                        .into_iter()
                        .map(ToolParam::into_wit)
                        .collect(),
                    scope: self.scope.into_wit(),
                }
            }
        }

        #[derive(Debug)]
        pub struct ToolParam {
            pub name: String,
            pub param_type: String,
            pub description: String,
        }

        impl ToolParam {
            #[must_use]
            pub fn new(name: impl Into<String>, param_type: impl Into<String>) -> Self {
                Self {
                    name: name.into(),
                    param_type: param_type.into(),
                    description: String::new(),
                }
            }

            pub fn described_as(mut self, description: impl Into<String>) -> Self {
                self.description = description.into();
                self
            }

            #[must_use]
            pub fn into_wit(self) -> WitToolParam {
                WitToolParam {
                    name: self.name,
                    param_type: self.param_type,
                    description: self.description,
                }
            }
        }

        #[derive(Debug, Clone, Copy)]
        pub enum ToolScope {
            Global,
            Attached,
        }

        impl ToolScope {
            #[must_use]
            pub fn into_wit(self) -> WitToolScope {
                match self {
                    Self::Global => WitToolScope::Global,
                    Self::Attached => WitToolScope::Attached,
                }
            }
        }
        /// Exhaustive set of theme style slots. A typo is a compile error here.
        ///
        /// Each maps to a theme field name the host resolves against the active theme.
        #[derive(Debug, Clone, Copy)]
        pub enum Style {
            Default,
            MutedText,
            ErrorText,
            Success,
            Warning,
            Streaming,
            AccentAction,
            PrimaryText,
        }

        impl Style {
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    Self::Default => "default",
                    Self::MutedText => "muted_text",
                    Self::ErrorText => "error_text",
                    Self::Success => "success",
                    Self::Warning => "warning",
                    Self::Streaming => "streaming",
                    Self::AccentAction => "accent_action",
                    Self::PrimaryText => "primary_text",
                }
            }

            #[must_use]
            pub fn as_opt(self) -> Option<String> {
                if matches!(self, Self::Default) {
                    None
                } else {
                    Some(self.as_str().to_owned())
                }
            }
        }

        /// A styled text run within a badge.
        #[derive(Debug)]
        pub struct Segment {
            pub text: String,
            pub style: Style,
        }

        impl Segment {
            #[must_use]
            pub fn text(text: impl Into<String>) -> Self {
                Self {
                    text: text.into(),
                    style: Style::Default,
                }
            }

            pub fn styled(text: impl Into<String>, style: Style) -> Self {
                Self {
                    text: text.into(),
                    style,
                }
            }

            pub fn muted(self) -> Self {
                Self {
                    style: Style::MutedText,
                    ..self
                }
            }

            pub fn streaming(self) -> Self {
                Self {
                    style: Style::Streaming,
                    ..self
                }
            }

            #[must_use]
            pub fn into_wit(self) -> BadgeSegment {
                BadgeSegment {
                    text: self.text,
                    style: self.style.as_opt(),
                }
            }
        }

        /// Builder for the `on-chat-input-badges-render` return directive.
        pub struct BadgeDirective;

        impl BadgeDirective {
            /// A badge drawn in the input-badge slot, left-to-right.
            #[must_use]
            pub fn input_badge(
                segments: impl IntoIterator<Item = Segment>,
            ) -> crate::jinn::plugin::types::BadgeDirective {
                crate::jinn::plugin::types::BadgeDirective {
                    slot: "input_badge".to_owned(),
                    segments: segments.into_iter().map(Segment::into_wit).collect(),
                }
            }
        }
    };
}

/// Emits `mod plugin { pub trait Plugin { ... } }` body.
#[macro_export]
macro_rules! __jinn_pdk_plugin_body {
    () => {
        /// The user-facing plugin trait. Override only the hooks you care about;
        /// defaults are no-ops. `plugin!($ty)` generates the wit-bindgen `Guest` impl
        /// that forwards each lifecycle/render hook to the corresponding `Plugin` method.
        ///
        /// `get_manifest` has no default — every plugin must declare itself.
        pub trait Plugin: Sized {
            fn get_manifest() -> crate::manifest::Manifest;

            async fn on_app_started(_ctx: crate::jinn::plugin::types::SessionCtx) {}
            async fn on_session_created(_ctx: crate::jinn::plugin::types::SessionCtx) {}
            async fn on_attach(_ctx: crate::jinn::plugin::types::AttachCtx) {}
            async fn on_detach(_ctx: crate::jinn::plugin::types::AttachCtx) {}
            async fn on_turn_end(_ctx: crate::jinn::plugin::types::TurnEndCtx) {}
            async fn on_user_submit(_ctx: crate::jinn::plugin::types::SessionCtx) {}
            async fn on_task_list_updated(_ctx: crate::jinn::plugin::types::TaskListCtx) {}

            /// Plugin-defined async trigger. `action` is the keybind's `action` string
            /// (e.g. `"on_enrich"`); the host passes it through verbatim and never
            /// hard-codes it. Override this and `match` on `action` to handle your
            /// keybind-triggered async hooks.
            async fn run_trigger(_action: String, _ctx: crate::jinn::plugin::types::TriggerCtx) {}

            /// Plugin-defined tool handler. `name` is the tool name (from the manifest);
            /// `args` is the raw JSON the LLM supplied. Override and `match` on `name`.
            ///
            /// Returns the tool result content (text fed back to the LLM). Empty string
            /// is a valid no-content response.
            async fn run_tool(
                _name: String,
                _args: String,
                _ctx: crate::jinn::plugin::types::ToolCtx,
            ) -> String {
                String::new()
            }

            fn on_chat_input_badges_render(
                _ctx: crate::jinn::plugin::types::BadgeCtx,
            ) -> Option<crate::jinn::plugin::types::BadgeDirective> {
                None
            }

            fn on_keybind_trigger(
                _ctx: crate::jinn::plugin::types::KeybindTriggerCtx,
            ) -> Option<crate::jinn::plugin::types::KeybindResult> {
                None
            }

            fn on_session_preview(
                _ctx: crate::jinn::plugin::types::SessionPreviewCtx,
            ) -> Option<String> {
                None
            }

            fn on_submit_intercept(
                _ctx: crate::jinn::plugin::types::SubmitInterceptCtx,
            ) -> Option<crate::jinn::plugin::types::InterceptOutcome> {
                None
            }
        }
    };
}

/// Generates the wit-bindgen `Guest` impl for `$ty`, forwarding each hook to the
/// `Plugin` trait method and converting `get_manifest` via `into_wit()`.
#[macro_export]
macro_rules! __jinn_pdk_guest_impl {
    ($ty:ident) => {
        impl crate::exports::jinn::plugin::hooks::Guest for $ty {
            fn get_manifest() -> crate::jinn::plugin::types::Manifest {
                <$ty as crate::plugin::Plugin>::get_manifest().into_wit()
            }

            async fn on_app_started(ctx: crate::jinn::plugin::types::SessionCtx) {
                <$ty as crate::plugin::Plugin>::on_app_started(ctx).await;
            }

            async fn on_session_created(ctx: crate::jinn::plugin::types::SessionCtx) {
                <$ty as crate::plugin::Plugin>::on_session_created(ctx).await;
            }

            async fn on_attach(ctx: crate::jinn::plugin::types::AttachCtx) {
                <$ty as crate::plugin::Plugin>::on_attach(ctx).await;
            }

            async fn on_detach(ctx: crate::jinn::plugin::types::AttachCtx) {
                <$ty as crate::plugin::Plugin>::on_detach(ctx).await;
            }

            async fn on_turn_end(ctx: crate::jinn::plugin::types::TurnEndCtx) {
                <$ty as crate::plugin::Plugin>::on_turn_end(ctx).await;
            }

            async fn on_user_submit(ctx: crate::jinn::plugin::types::SessionCtx) {
                <$ty as crate::plugin::Plugin>::on_user_submit(ctx).await;
            }

            async fn on_task_list_updated(ctx: crate::jinn::plugin::types::TaskListCtx) {
                <$ty as crate::plugin::Plugin>::on_task_list_updated(ctx).await;
            }

            async fn run_trigger(action: String, ctx: crate::jinn::plugin::types::TriggerCtx) {
                <$ty as crate::plugin::Plugin>::run_trigger(action, ctx).await;
            }

            async fn run_tool(
                name: String,
                args: String,
                ctx: crate::jinn::plugin::types::ToolCtx,
            ) -> String {
                <$ty as crate::plugin::Plugin>::run_tool(name, args, ctx).await
            }
            fn on_chat_input_badges_render(
                ctx: crate::jinn::plugin::types::BadgeCtx,
            ) -> Option<crate::jinn::plugin::types::BadgeDirective> {
                <$ty as crate::plugin::Plugin>::on_chat_input_badges_render(ctx)
            }

            fn on_keybind_trigger(
                ctx: crate::jinn::plugin::types::KeybindTriggerCtx,
            ) -> Option<crate::jinn::plugin::types::KeybindResult> {
                <$ty as crate::plugin::Plugin>::on_keybind_trigger(ctx)
            }

            fn on_session_preview(
                ctx: crate::jinn::plugin::types::SessionPreviewCtx,
            ) -> Option<String> {
                <$ty as crate::plugin::Plugin>::on_session_preview(ctx)
            }

            fn on_submit_intercept(
                ctx: crate::jinn::plugin::types::SubmitInterceptCtx,
            ) -> Option<crate::jinn::plugin::types::InterceptOutcome> {
                <$ty as crate::plugin::Plugin>::on_submit_intercept(ctx)
            }
        }
    };
}
