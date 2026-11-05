//! Every method name of version 1, exactly as it travels on the wire.

pub const INITIALIZE: &str = "initialize";
pub const SHUTDOWN: &str = "shutdown";
pub const WORKSPACE_OPEN: &str = "workspace/open";
pub const STATUS_GET: &str = "status/get";
pub const CONNECTION_CLOSE: &str = "connection/close";
pub const SERVICE_CONTROL: &str = "service/control";
pub const RELEASES_LIST: &str = "releases/list";
pub const RELEASE_CONTENTS: &str = "release/contents";
pub const RELEASE_READ_FILE: &str = "release/read_file";
pub const LOGS_FOLLOW: &str = "logs/follow";
pub const LOGS_STOP: &str = "logs/stop";
pub const DEPLOY_START: &str = "deploy/start";
pub const ROLLBACK_START: &str = "rollback/start";

pub const LOGS_LINE: &str = "logs/line";
pub const LOGS_ENDED: &str = "logs/ended";
pub const OP_STEP: &str = "op/step";
pub const OP_PROGRESS: &str = "op/progress";
pub const OP_LOG: &str = "op/log";
pub const OP_RELEASE: &str = "op/release";
pub const OP_FINISHED: &str = "op/finished";

pub const ASK_ROLLBACK: &str = "ask/rollback";
pub const ASK_UNCHANGED: &str = "ask/unchanged";
pub const ASK_SECRET: &str = "ask/secret";
pub const ASK_TRUST_HOST: &str = "ask/trust_host";
