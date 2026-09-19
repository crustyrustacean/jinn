//! The session-init slice's bridge drain: schema→topic route rules.
//!
//! Topics are named here (this slice consumes them); the trouper fabric
//! dispatches by schema id, so the drain only needs to register the
//! route rules — no relay actors.

use jinn_domain::Services;

use crate::session_init_topic;

/// Registers the session-init slice's schema→topic route rules: the 7
/// forward triggers onto [`session_init_topic`]. The 3 discovery
/// results (`SkillsLoaded`, `PromptTemplatesLoaded`,
/// `ContextFilesLoaded`) publish onto the shared kernel topic — the
/// worker's `publish` helper resolves the same default — so no route
/// override exists for them.
pub fn install_topic_routes(services: &Services) {
    services.bus.route_topic::<jinn_domain::feat::session_lifecycle::protocol::event::SessionCreated>(session_init_topic());
    services.bus.route_topic::<jinn_session_msg::SessionSetupCompleted>(session_init_topic());
    services.bus.route_topic::<jinn_domain::feat::session::protocol::session_load_completed::SessionLoadCompleted>(session_init_topic());
    services
        .bus
        .route_topic::<jinn_domain::feat::session_lifecycle::protocol::event::SessionCwdChanged>(
            session_init_topic(),
        );
    services.bus.route_topic::<jinn_skills_msg::ScanSkills>(session_init_topic());
    services.bus.route_topic::<jinn_domain::feat::provider::protocol::command::RescanPromptTemplates>(session_init_topic());
    services.bus.route_topic::<jinn_domain::feat::context::protocol::command::ScanContextFiles>(session_init_topic());

}

