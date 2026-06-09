use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Json, Response,
    },
    routing::{get, post},
    Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use sigmacode_core::agent::engine::Agent;
use sigmacode_core::context::ContextBuilder;
use sigmacode_core::llm::{LlmProvider, OpenAiProvider};
use sigmacode_core::tools::ToolRouter;
use sigmacode_core::types::{
    AgentConfig, AgentEvent, AgentMode, AgentState, Session, SessionMessage, WorkingMemory,
};

fn sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sigma")
        .join("sessions")
}

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    server_key: Option<String>,
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    workspace: Option<String>,
    #[serde(default)]
    mode: AgentMode,
    session_id: Option<String>,
}

#[derive(Serialize)]
struct ChatResponse {
    response: String,
    message_count: usize,
    session_id: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    service: String,
}

#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    models: Vec<String>,
}

#[derive(Serialize)]
struct ConfigResponse {
    providers: Vec<ProviderInfo>,
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    title: String,
    mode: String,
    created_at: String,
    updated_at: String,
    message_count: usize,
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    title: Option<String>,
    mode: Option<AgentMode>,
}

#[derive(Deserialize)]
struct AddMessageRequest {
    role: String,
    content: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: "0.1.0".into(),
        service: "sigmacode-server".into(),
    })
}

async fn config() -> Json<ConfigResponse> {
    Json(ConfigResponse {
        providers: vec![
            ProviderInfo {
                name: "openai".into(),
                models: vec!["gpt-4o".into(), "gpt-4o-mini".into()],
            },
            ProviderInfo {
                name: "anthropic".into(),
                models: vec![
                    "claude-sonnet-4-20250514".into(),
                    "claude-3-5-haiku-20241022".into(),
                ],
            },
            ProviderInfo {
                name: "ollama".into(),
                models: vec!["llama3".into(), "codellama".into()],
            },
        ],
    })
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(ref required_key) = state.server_key {
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let provided_key = &header[7..];
                if provided_key != required_key.as_str() {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
            _ => {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    Ok(next.run(req).await)
}

// ── Session Management ──

fn load_session(id: &str) -> Option<Session> {
    let path = sessions_dir().join(format!("{}.json", id));
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_session(session: &Session) -> anyhow::Result<()> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", session.id));
    let content = serde_json::to_string_pretty(session)?;
    std::fs::write(path, content)?;
    Ok(())
}

fn list_sessions_from_disk() -> Vec<SessionInfo> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Vec::new();
    }

    let mut sessions: Vec<SessionInfo> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|e| {
            let content = std::fs::read_to_string(e.path()).ok()?;
            let session: Session = serde_json::from_str(&content).ok()?;
            Some(SessionInfo {
                id: session.id,
                title: session.title,
                mode: session.mode.to_string(),
                created_at: session.created_at,
                updated_at: session.updated_at,
                message_count: session.messages.len(),
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

async fn list_sessions() -> Json<Vec<SessionInfo>> {
    Json(list_sessions_from_disk())
}

async fn create_session(Json(req): Json<CreateSessionRequest>) -> Json<SessionInfo> {
    let now = chrono::Utc::now().to_rfc3339();
    let session = Session {
        id: Uuid::new_v4().to_string(),
        title: req.title.unwrap_or_else(|| "New session".into()),
        mode: req.mode.unwrap_or_default(),
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
    };

    let info = SessionInfo {
        id: session.id.clone(),
        title: session.title.clone(),
        mode: session.mode.to_string(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        message_count: 0,
    };

    let _ = save_session(&session);
    Json(info)
}

async fn get_session(Path(id): Path<String>) -> Result<Json<Session>, StatusCode> {
    load_session(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn delete_session(Path(id): Path<String>) -> Result<StatusCode, StatusCode> {
    let path = sessions_dir().join(format!("{}.json", id));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn add_message(
    Path(id): Path<String>,
    Json(req): Json<AddMessageRequest>,
) -> Result<StatusCode, StatusCode> {
    let mut session = load_session(&id).ok_or(StatusCode::NOT_FOUND)?;
    let now = chrono::Utc::now().to_rfc3339();

    session.messages.push(SessionMessage {
        role: req.role,
        content: req.content,
        timestamp: now,
    });
    session.updated_at = chrono::Utc::now().to_rfc3339();

    save_session(&session).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

// ── Chat ──

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    let workspace = req
        .workspace
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let session_id = req
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut state_agent = AgentState {
        session_id: Uuid::parse_str(&session_id).unwrap_or_else(|_| Uuid::new_v4()),
        task: req.message.clone(),
        messages: Vec::new(),
        plan: None,
        results: Vec::new(),
        working_memory: WorkingMemory::new(8000),
        workspace,
        config: AgentConfig::default(),
        iteration: 0,
        event_tx: None,
    };

    let cancel = tokio_util::sync::CancellationToken::new();

    match state
        .agent
        .run_with_mode(&mut state_agent, cancel, None, &req.mode)
        .await
    {
        Ok(response) => {
            let message_count = state_agent.messages.len();

            // Save messages to session
            let mut session = load_session(&session_id).unwrap_or_else(|| Session {
                id: session_id.clone(),
                title: req.message.chars().take(60).collect(),
                mode: req.mode,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                messages: Vec::new(),
            });

            let now = chrono::Utc::now().to_rfc3339();
            session.messages.push(SessionMessage {
                role: "user".into(),
                content: req.message,
                timestamp: now.clone(),
            });
            session.messages.push(SessionMessage {
                role: "assistant".into(),
                content: response.clone(),
                timestamp: now,
            });
            session.updated_at = chrono::Utc::now().to_rfc3339();
            let _ = save_session(&session);

            Ok(Json(ChatResponse {
                response,
                message_count,
                session_id,
            }))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn chat_stream(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let workspace = req
        .workspace
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let session_id = req
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mode = req.mode.clone();

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();

    let agent = state.agent.clone();
    let message = req.message.clone();

    tokio::spawn(async move {
        let mut state_agent = AgentState {
            session_id: Uuid::parse_str(&session_id).unwrap_or_else(|_| Uuid::new_v4()),
            task: message.clone(),
            messages: Vec::new(),
            plan: None,
            results: Vec::new(),
            working_memory: WorkingMemory::new(8000),
            workspace,
            config: AgentConfig::default(),
            iteration: 0,
            event_tx: Some(tx.clone()),
        };

        let cancel = tokio_util::sync::CancellationToken::new();

        let result = agent
            .run_with_mode(&mut state_agent, cancel, Some(tx.clone()), &mode)
            .await;

        // Save to session
        if let Ok(response) = &result {
            let mut session = load_session(&session_id).unwrap_or_else(|| Session {
                id: session_id.clone(),
                title: message.chars().take(60).collect(),
                mode,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                messages: Vec::new(),
            });

            let now = chrono::Utc::now().to_rfc3339();
            session.messages.push(SessionMessage {
                role: "user".into(),
                content: message,
                timestamp: now.clone(),
            });
            session.messages.push(SessionMessage {
                role: "assistant".into(),
                content: response.clone(),
                timestamp: now,
            });
            session.updated_at = chrono::Utc::now().to_rfc3339();
            let _ = save_session(&session);
        }
    });

    let stream = async_stream::stream! {
        while let Some(event) = rx.recv().await {
            let data = serde_json::to_string(&event).unwrap_or_default();
            yield Ok(Event::default().data(data));
        }
        yield Ok(Event::default().data("[DONE]"));
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sigmacode_server=info,tower_http=info".into()),
        )
        .init();

    let port: u16 = std::env::var("SIGMACODE_PORT")
        .unwrap_or_else(|_| "3847".into())
        .parse()
        .unwrap_or(3847);

    let api_key = std::env::var("SIGMACODE_API_KEY")
        .unwrap_or_else(|_| "sk-sr80smbaiismtvgf52xgfthezp2qy4c9s7m7vqdsih2wnij5".into());

    let base_url = std::env::var("SIGMACODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.xiaomimimo.com/v1".into());

    let model = std::env::var("SIGMACODE_MODEL")
        .unwrap_or_else(|_| "mimo-v2.5".into());

    let server_key = {
        // 1. If SERVER_KEY env var is set explicitly, use it directly
        if let Some(key) = std::env::var("SERVER_KEY").ok().filter(|k| !k.is_empty()) {
            tracing::info!("Using SERVER_KEY from environment variable");
            Some(key)
        } else {
            // 2. Try loading from Redis (encrypted) only if ENCRYPTION_KEY is set
            let encryption_key = std::env::var("ENCRYPTION_KEY").ok();
            if let Some(ref master_key) = encryption_key {
                let redis_url = std::env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
                match sigmacode_core::key_store::load_key_from_redis(master_key, &redis_url).await {
                    Ok(key) => {
                        tracing::info!("Loaded SERVER_KEY from Redis (encrypted)");
                        Some(key)
                    }
                    Err(e) => {
                        tracing::warn!("Redis load failed: {} — running without auth", e);
                        None
                    }
                }
            } else {
                None
            }
        }
    };

    if server_key.is_some() {
        tracing::info!("SERVER_KEY is set — API routes require Authorization header");
    } else {
        tracing::info!("SERVER_KEY not set — API routes are open (local dev mode)");
    }

    tracing::info!("Starting sigmacode-server on port {}", port);

    let provider: Box<dyn LlmProvider> = Box::new(OpenAiProvider::new(api_key, base_url, model));

    let mut tools = ToolRouter::new();
    tools.register_defaults();

    let context_builder = ContextBuilder::new("sigmacode-server");

    let agent = Agent::new(provider, tools, context_builder);

    let state = AppState {
        agent: Arc::new(agent),
        server_key,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/api/chat", post(chat))
        .route("/api/chat/stream", post(chat_stream))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/sessions/{id}/messages", post(add_message))
        .route("/config", get(config))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health))
        .merge(api_routes)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
