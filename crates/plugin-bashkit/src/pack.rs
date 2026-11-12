//! The `bash` Pack: its [`INFO`], and [`BashPack`], one run's instance,
//! which lists the Pack's one tool and holds the run's shell session.

use std::sync::Arc;

use runvil_plugin_sdk::{
    BoxFuture, Pack, PackInfo, ToolCall, ToolError, ToolErrorKind, ToolInfo, ToolOutput,
};
use tokio::runtime::Handle;
use tokio::sync::Mutex;

use crate::session::Session;
use crate::spec::Definition;

/// The Pack's name and description. Its name is `runvil/bash`, so its
/// Pack id is `bash`, and a prompt reaches its tool as `bash/shell`.
pub(crate) const INFO: PackInfo =
    PackInfo::new("runvil/bash").description("Run commands in a bash shell inside the run");

/// The tool's name within the Pack, the last segment of its Lua id.
const SHELL: &str = "shell";

/// One run's instance of the Pack: the tool's definition, which every run
/// shares, and the run's own shell session.
pub(crate) struct BashPack {
    definition: Arc<Definition>,
    /// The Plugin's runtime, which runs every shell's timers.
    timers: Handle,
    session: Session,
}

impl BashPack {
    /// Makes the instance, with no shell yet: the first call makes it.
    pub(crate) fn new(definition: Arc<Definition>, timers: Handle) -> BashPack {
        BashPack {
            definition,
            timers,
            session: Mutex::new(None),
        }
    }
}

impl Pack for BashPack {
    fn tools(&self) -> Vec<ToolInfo> {
        let definition = &self.definition;
        vec![
            ToolInfo::new(
                SHELL,
                definition.description.clone(),
                definition.parameters.clone(),
            )
            .wire_name(definition.wire_name.clone()),
        ]
    }

    fn call<'a>(&'a self, call: ToolCall<'a>) -> BoxFuture<'a, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            if call.tool() != SHELL {
                return Err(ToolError::new(
                    ToolErrorKind::Other,
                    format!("bash has no tool named {}", call.tool()),
                ));
            }
            crate::shell::call(&self.session, &self.timers, call).await
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::fixture::{PACK, loaded};

    #[test]
    fn the_plugin_inserts_the_one_pack_runvil_bash() {
        let loaded = loaded();
        let packs: Vec<(&str, &str)> = loaded
            .packs()
            .iter()
            .map(|pack| (pack.name.as_str(), pack.description.as_str()))
            .collect();
        assert_eq!(
            packs,
            [(PACK, "Run commands in a bash shell inside the run")]
        );
    }

    #[test]
    fn an_instance_adds_the_one_tool_shell_under_its_wire_name() {
        let loaded = loaded();
        let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
        let names: Vec<(&str, Option<&str>)> = pack
            .tools()
            .iter()
            .map(|tool| (tool.name.as_str(), tool.wire_name.as_deref()))
            .collect();
        assert_eq!(names, [("shell", Some("Shell"))]);
    }

    #[test]
    fn each_run_gets_an_instance_of_its_own_with_the_same_tool() {
        let loaded = loaded();
        let first = loaded.new_pack(PACK).expect("the Pack instance is made");
        let second = loaded.new_pack(PACK).expect("the Pack instance is made");
        assert_eq!(first.tools(), second.tools());
    }
}
