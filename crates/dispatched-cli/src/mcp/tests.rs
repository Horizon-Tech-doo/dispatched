//! The server end to end, in process, through rmcp's client over a duplex pipe. The client
//! answers elicitations as the test says. The hermetic tests stop at the gate (a denial needs
//! no broker); `mcp_emulator` runs reads and changes against the emulator, or the cloud
//! namespace of `DISPATCHED_GATE_CONNECTION_STRING`.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched mcp_emulator -- --nocapture`

use super::Server;
use super::policy::NO_FORMS;
use crate::context::Ctx;
use crate::resolve::Selection;
use dispatched_core::activity::{ActivityQuery, Outcome};
use horizon_azure_kit::safety::Origin;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ElicitRequestParams,
    ElicitResult, ElicitationAction, ElicitationCapability, FormElicitationCapability,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceError, ServiceExt};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

/// How the test client answers the next forms.
#[derive(Debug, Clone)]
enum Mode {
    /// Tick the checkbox, or type this name on a production form.
    Accept(String),
    Decline,
    Cancel,
}

/// A client that declares form elicitation (or not) and answers by `mode`, recording the
/// messages it was shown.
#[derive(Clone)]
struct TestClient {
    forms: bool,
    mode: Arc<Mutex<Mode>>,
    shown: Arc<Mutex<Vec<String>>>,
}

impl TestClient {
    fn new(forms: bool, mode: Mode) -> Self {
        Self {
            forms,
            mode: Arc::new(Mutex::new(mode)),
            shown: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn shown(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }
}

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        let mut info = ClientConfig::default();
        if self.forms {
            info.capabilities = ClientCapabilities::builder()
                .enable_elicitation_with(
                    ElicitationCapability::new().with_form(FormElicitationCapability::new()),
                )
                .build();
        }
        info
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, rmcp::ErrorData> {
        let ElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } = request
        else {
            return Ok(ElicitResult::new(ElicitationAction::Decline));
        };
        self.shown.lock().unwrap().push(message);
        let mode = self.mode.lock().unwrap().clone();
        Ok(match mode {
            Mode::Accept(name) => {
                let content = if requested_schema.properties.contains_key("name") {
                    json!({ "name": name })
                } else {
                    json!({ "confirm": true })
                };
                ElicitResult::new(ElicitationAction::Accept).with_content(content)
            }
            Mode::Decline => ElicitResult::new(ElicitationAction::Decline),
            Mode::Cancel => ElicitResult::new(ElicitationAction::Cancel),
        })
    }
}

/// A server over `ctx` on `cs`, and a connected client.
async fn connect(
    ctx: Ctx,
    cs: &str,
    read_only: bool,
    client: TestClient,
) -> (Server, RunningService<RoleClient, TestClient>) {
    let sel = Selection {
        connection_string: Some(cs),
        namespace: None,
        profile: None,
    };
    let server = Server::new(ctx, &sel, read_only);
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let serving = server.clone();
    tokio::spawn(async move {
        if let Ok(running) = serving.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = client.serve(client_io).await.expect("client connects");
    (server, client)
}

fn args(v: Value) -> rmcp::model::JsonObject {
    v.as_object().cloned().expect("an object")
}

async fn call(
    client: &RunningService<RoleClient, TestClient>,
    tool: &'static str,
    arguments: Value,
) -> Result<CallToolResult, ServiceError> {
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args(arguments)))
        .await
}

/// The text of a result (its first content block).
fn text(r: &CallToolResult) -> String {
    r.content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

/// The structured JSON of a successful result.
fn ok_json(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "{}", text(r));
    r.structured_content.clone().expect("structured content")
}

fn is_error(r: &CallToolResult, needle: &str) -> bool {
    r.is_error == Some(true) && text(r).contains(needle)
}

const READ_TOOLS: [&str; 10] = [
    "diagnose_subscription",
    "get_counts",
    "get_dlq_summary",
    "get_entity",
    "get_session_state",
    "list_entities",
    "list_profiles",
    "list_sessions",
    "peek_messages",
    "search_messages",
];

const WRITE_TOOLS: [&str; 11] = [
    "cancel_scheduled",
    "delete_messages",
    "entity_create",
    "entity_delete",
    "entity_update",
    "move_messages",
    "purge",
    "resubmit_dlq",
    "schedule_message",
    "send_message",
    "set_session_state",
];

#[tokio::test]
async fn read_only_lists_only_the_read_tools() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let (_server, client) =
        connect(ctx, EMULATOR, true, TestClient::new(true, Mode::Decline)).await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, READ_TOOLS);
    for t in &tools {
        let a = t.annotations.as_ref().expect("annotated");
        assert_eq!(a.read_only_hint, Some(true), "{}", t.name);
    }
    // A write tool is not there to call.
    assert!(
        call(&client, "purge", json!({"entity": "orders"}))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_full_list_has_every_write_tool_annotated() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let (_server, client) =
        connect(ctx, EMULATOR, false, TestClient::new(true, Mode::Decline)).await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    let mut all: Vec<&str> = READ_TOOLS
        .iter()
        .chain(WRITE_TOOLS.iter())
        .copied()
        .collect();
    all.sort();
    assert_eq!(names, all);
    for t in tools
        .iter()
        .filter(|t| WRITE_TOOLS.contains(&t.name.as_ref()))
    {
        let a = t.annotations.as_ref().expect("annotated");
        assert_eq!(a.read_only_hint, Some(false), "{}", t.name);
        let destructive = !matches!(
            t.name.as_ref(),
            "send_message" | "schedule_message" | "entity_create"
        );
        assert_eq!(a.destructive_hint, Some(destructive), "{}", t.name);
    }
    // No tool takes a connection string.
    for t in &tools {
        let schema = serde_json::to_string(&t.input_schema).unwrap();
        assert!(!schema.contains("connection_string"), "{}", t.name);
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn list_profiles_shows_no_secret() {
    let ctx = Ctx::for_test_with_profiles(&[("prod-weu", "prod.servicebus.windows.net", true)]);
    let secret = "Endpoint=sb://dev.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=topsecretvalue=";
    {
        let store = crate::context::StoreSource::store(&ctx).unwrap();
        let cs = dispatched_core::ConnectionString::parse(secret).unwrap();
        let p = dispatched_core::profiles::Profile::connection_string("dev", &cs);
        store.insert(&p).unwrap();
        ctx.secrets
            .put(
                &dispatched_core::profiles::secret_key(&p.id),
                &secrecy::SecretString::from(secret),
            )
            .unwrap();
        store.set_default(Some("dev")).unwrap();
    }
    let (_server, client) = connect(ctx, secret, true, TestClient::new(true, Mode::Decline)).await;
    let r = call(&client, "list_profiles", json!({})).await.unwrap();
    let raw = serde_json::to_string(&r).unwrap();
    assert!(!raw.contains("topsecretvalue"), "{raw}");
    assert!(!raw.contains("SharedAccessKey"), "{raw}");
    let v = ok_json(&r);
    let profiles = v["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    let dev = profiles.iter().find(|p| p["name"] == "dev").unwrap();
    assert_eq!(dev["auth"], "connection-string");
    assert_eq!(dev["default"], true);
    let prod = profiles.iter().find(|p| p["name"] == "prod-weu").unwrap();
    assert_eq!(prod["production"], true);
    assert_eq!(prod["auth"], "entra");
    assert_eq!(v["server_default"]["host"], "dev.servicebus.windows.net");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bad_arguments_are_protocol_errors() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let (_server, client) =
        connect(ctx, EMULATOR, false, TestClient::new(true, Mode::Decline)).await;
    for (tool, a) in [
        ("peek_messages", json!({"entity": "orders", "top": 501})),
        (
            "peek_messages",
            json!({"entity": "orders", "sub_queue": "nope"}),
        ),
        ("search_messages", json!({"entity": "orders"})),
        ("peek_messages", json!({"entity": "a//b"})),
        ("delete_messages", json!({"entity": "orders"})),
        ("send_message", json!({"entity": "orders", "count": 1000})),
        (
            "set_session_state",
            json!({"entity": "s", "session": "a", "text": "x", "clear": true}),
        ),
        (
            "entity_update",
            json!({"kind": "queue", "entity": "orders", "settings": {}}),
        ),
        (
            "entity_update",
            json!({"kind": "queue", "entity": "orders", "settings": {"requires_session": true}}),
        ),
    ] {
        match call(&client, tool, a.clone()).await {
            Err(ServiceError::McpError(e)) => {
                assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{tool} {a}")
            }
            // rmcp returns arguments that do not match the schema as a tool error, so the
            // model can correct itself.
            Ok(r) if is_error(&r, "failed to deserialize parameters") => {}
            other => panic!("{tool} {a}: expected invalid params, got {other:?}"),
        }
    }
    client.cancel().await.unwrap();
}

/// The activity entries the server wrote.
fn activity(server: &Server) -> Vec<dispatched_core::ActivityEntry> {
    server
        .ctx()
        .activity()
        .unwrap()
        .list(&ActivityQuery::default())
        .unwrap()
}

#[tokio::test]
async fn a_client_without_forms_is_refused_and_the_refusal_is_logged() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let client = TestClient::new(false, Mode::Accept(String::new()));
    let (server, client) = connect(ctx, EMULATOR, false, client).await;
    let r = call(
        &client,
        "send_message",
        json!({"entity": "orders", "body": "x"}),
    )
    .await
    .unwrap();
    assert!(is_error(&r, NO_FORMS), "{}", text(&r));
    let entries = activity(&server);
    assert_eq!(entries.len(), 1, "{entries:?}");
    let e = &entries[0];
    assert_eq!(e.origin, Origin::Mcp);
    assert_eq!(e.outcome, Outcome::Denied);
    assert_eq!(e.action, "send");
    assert_eq!(e.profile, "localhost");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn declined_and_cancelled_forms_change_nothing() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let tc = TestClient::new(true, Mode::Decline);
    let (server, client) = connect(ctx, EMULATOR, false, tc.clone()).await;
    let r = call(
        &client,
        "send_message",
        json!({"entity": "orders", "body": "x", "count": 3}),
    )
    .await
    .unwrap();
    assert!(is_error(&r, "declined"), "{}", text(&r));
    let shown = tc.shown();
    assert_eq!(shown.len(), 1);
    assert!(
        shown[0].starts_with("Send 3 messages to orders"),
        "{}",
        shown[0]
    );
    assert!(shown[0].contains("Items affected: 3."), "{}", shown[0]);

    tc.set(Mode::Cancel);
    let r = call(
        &client,
        "cancel_scheduled",
        json!({"entity": "orders", "sequence_numbers": [5, 6]}),
    )
    .await
    .unwrap();
    assert!(is_error(&r, "cancelled"), "{}", text(&r));
    let entries = activity(&server);
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|e| e.origin == Origin::Mcp && e.outcome == Outcome::Denied),
        "{entries:?}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn production_is_read_from_the_saved_profiles_and_needs_the_typed_name() {
    // The connection string's host is saved as a production profile: the agent cannot say
    // otherwise.
    let ctx = Ctx::for_test_with_profiles(&[("emu-prod", "localhost", true)]);
    let tc = TestClient::new(true, Mode::Accept("order".into()));
    let (server, client) = connect(ctx, EMULATOR, false, tc.clone()).await;
    let r = call(
        &client,
        "send_message",
        json!({"entity": "orders", "body": "x"}),
    )
    .await
    .unwrap();
    assert!(is_error(&r, "does not match"), "{}", text(&r));
    let shown = tc.shown();
    assert!(
        shown[0].ends_with("This connection is marked as production. Type orders to confirm."),
        "{}",
        shown[0]
    );
    let entries = activity(&server);
    assert_eq!(entries[0].outcome, Outcome::Denied);
    assert!(
        entries[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("does not match")
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_unknown_profile_or_missing_default_is_a_readable_error() {
    let ctx = Ctx::for_test_with_profiles(&[]);
    let (_server, client) =
        connect(ctx, EMULATOR, true, TestClient::new(true, Mode::Decline)).await;
    let r = call(
        &client,
        "get_entity",
        json!({"entity": "orders", "profile": "nope"}),
    )
    .await
    .unwrap();
    assert!(is_error(&r, "no profile named `nope`"), "{}", text(&r));
    client.cancel().await.unwrap();

    // No selection and no default profile: the server still starts, and says what to pass.
    let ctx = Ctx::for_test_with_profiles(&[]);
    let server = Server::new(
        ctx,
        &Selection {
            connection_string: None,
            namespace: None,
            profile: None,
        },
        true,
    );
    let e = server.target(None).err().unwrap().to_string();
    assert!(
        e.contains("no namespace selected") && e.contains("pass `profile`"),
        "{e}"
    );
}

#[test]
fn bodies_are_cut_at_64_kib_and_binary_is_base64() {
    use super::read::{BODY_CAP, body_json, cut_text};
    use dispatched_core::MessageBody;
    let big = "é".repeat(BODY_CAP);
    let v = body_json(&MessageBody::Text(big.clone()));
    assert_eq!(v["kind"], "text");
    assert_eq!(v["truncated"], true);
    assert_eq!(v["size"], big.len());
    assert!(v["text"].as_str().unwrap().len() <= BODY_CAP);
    assert_eq!(cut_text("abc", 2), ("ab", true));
    assert_eq!(cut_text("é", 1), ("", true));

    let v = body_json(&MessageBody::Binary(b"{\"a\":1}".to_vec()));
    assert_eq!(v["kind"], "binary");
    assert_eq!(v["text"], "{\"a\":1}");
    assert_eq!(v["truncated"], false);

    let v = body_json(&MessageBody::Binary(vec![0xff; BODY_CAP + 1]));
    assert_eq!(v["truncated"], true);
    assert!(v.get("text").is_none());
    assert_eq!(v["size"], BODY_CAP + 1);
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(v["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded.len(), BODY_CAP);

    let v = body_json(&MessageBody::Empty);
    assert_eq!(v["kind"], "empty");
    assert_eq!(v["size"], 0);
}

// ---------------------------------------------------------------------------------------------
// Against the emulator (or a cloud test namespace)

fn gate_cs() -> String {
    std::env::var("DISPATCHED_GATE_CONNECTION_STRING").unwrap_or_else(|_| EMULATOR.to_string())
}

fn emulator_enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

/// Deletes the test queue on every path.
struct QueueGuard {
    mgmt: dispatched_core::ManagementClient,
    name: String,
}

impl Drop for QueueGuard {
    fn drop(&mut self) {
        let mgmt = self.mgmt.clone();
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = rt.block_on(mgmt.delete_queue(&name));
        })
        .join();
    }
}

/// Peek until the queue holds `n` active messages (a new queue and a send take a moment to show).
async fn wait_for(client: &RunningService<RoleClient, TestClient>, queue: &str, n: usize) -> Value {
    let mut last = Value::Null;
    for _ in 0..30 {
        let r = call(client, "peek_messages", json!({"entity": queue}))
            .await
            .unwrap();
        if r.is_error != Some(true) {
            last = ok_json(&r);
            if last["messages"].as_array().map(Vec::len) == Some(n) {
                return last;
            }
        } else {
            last = json!({ "error": text(&r) });
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("{queue} did not reach {n} messages: {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_emulator() {
    if !emulator_enabled() {
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let cs = gate_cs();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let queue = format!("m4a-{}", &run[..8]);
    let ns = dispatched_core::Namespace::from_connection_string(
        &dispatched_core::ConnectionString::parse(&cs).unwrap(),
    )
    .unwrap();
    let _guard = QueueGuard {
        mgmt: dispatched_core::ManagementClient::new(&ns).unwrap(),
        name: queue.clone(),
    };

    let ctx = Ctx::for_test_with_profiles(&[]);
    let tc = TestClient::new(true, Mode::Accept(String::new()));
    let (server, client) = connect(ctx, &cs, false, tc.clone()).await;

    // The tool list.
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), READ_TOOLS.len() + WRITE_TOOLS.len());

    // Diagnose a subscription of Config.json (the emulator cannot list its rules, which the
    // findings then say).
    let r = call(
        &client,
        "diagnose_subscription",
        json!({"topic": "events", "subscription": "all"}),
    )
    .await
    .unwrap();
    let v = ok_json(&r);
    eprintln!("diagnose events/all: {}", v["findings"]);
    assert!(!v["findings"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(v["max_delivery_count"], 3, "{v}");
    let r = call(
        &client,
        "diagnose_subscription",
        json!({"topic": "events", "subscription": "missing"}),
    )
    .await
    .unwrap();
    assert!(
        r.is_error == Some(true),
        "a missing subscription is an error: {}",
        text(&r)
    );

    // Create the queue through the tool (accepted).
    let r = call(
        &client,
        "entity_create",
        json!({"kind": "queue", "entity": queue, "settings": {"lock_duration": "30s"}}),
    )
    .await
    .unwrap();
    let v = ok_json(&r);
    assert_eq!(v["succeeded"], 1, "{v}");
    let empty = wait_for(&client, &queue, 0).await;
    assert_eq!(empty["messages"], json!([]));

    // Send three (accepted), then peek them: nothing is locked or removed.
    let r = call(
        &client,
        "send_message",
        json!({"entity": queue, "body": "{\"n\":1}", "content_type": "application/json",
               "properties": ["run=x"], "count": 3}),
    )
    .await
    .unwrap();
    assert_eq!(ok_json(&r)["succeeded"], 3);
    let page = wait_for(&client, &queue, 3).await;
    let m = &page["messages"][0];
    assert_eq!(m["body"]["text"], "{\"n\":1}", "{m}");
    assert_eq!(m["body"]["truncated"], false);
    assert_eq!(m["delivery_count"], 0, "peeking locks nothing");
    let r = call(
        &client,
        "search_messages",
        json!({"entity": queue, "filter": "user.run = 'x'", "top": 2}),
    )
    .await
    .unwrap();
    assert_eq!(ok_json(&r)["messages"].as_array().unwrap().len(), 2);

    // Purge declined: nothing changes, and the log says mcp, denied.
    tc.set(Mode::Decline);
    let r = call(&client, "purge", json!({"entity": queue}))
        .await
        .unwrap();
    assert!(is_error(&r, "declined"), "{}", text(&r));
    wait_for(&client, &queue, 3).await;
    let denied = activity(&server)
        .into_iter()
        .find(|e| e.action == "purge" && e.outcome == Outcome::Denied)
        .expect("the declined purge is logged");
    assert_eq!(denied.origin, Origin::Mcp);
    assert_eq!(denied.entity.as_deref(), Some(queue.as_str()));

    // Purge accepted: the messages are gone.
    tc.set(Mode::Accept(String::new()));
    let r = call(&client, "purge", json!({"entity": queue}))
        .await
        .unwrap();
    let v = ok_json(&r);
    assert_eq!(v["succeeded"], 3, "{v}");
    wait_for(&client, &queue, 0).await;
    let purged = activity(&server)
        .into_iter()
        .filter(|e| e.action == "purge" && e.outcome == Outcome::Succeeded)
        .count();
    assert_eq!(purged, 1);

    // The shown sentences are the core's.
    let shown = tc.shown();
    assert!(
        shown
            .iter()
            .any(|s| s.starts_with("Remove ") && s.contains(&format!("3 messages from {queue}"))),
        "{shown:?}"
    );

    // Delete the queue through the tool; the guard covers a failure before this.
    let r = call(
        &client,
        "entity_delete",
        json!({"kind": "queue", "entity": queue}),
    )
    .await
    .unwrap();
    assert_eq!(ok_json(&r)["succeeded"], 1);
    client.cancel().await.unwrap();
    server.close().await;
    eprintln!("mcp_emulator: all steps passed");
}
