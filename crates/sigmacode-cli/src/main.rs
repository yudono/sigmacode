use sigmacode_core::llm::create_provider;
use sigmacode_core::tools::ToolRouter;
use sigmacode_core::Agent;
use sigmacode_core::AgentConfig;
use sigmacode_core::AgentState;
use sigmacode_core::ContextBuilder;
use sigmacode_core::ProviderConfig;
use sigmacode_core::WorkingMemory;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    let config = load_config()?;

    let task = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        eprintln!("Usage: sigmacode-headless <task>");
        eprintln!("Example: sigmacode-headless \"Add Google OAuth to the app\"");
        std::process::exit(1);
    };

    eprintln!("SigmaCode (headless) - Task: {}", task);

    let provider = create_provider(&config.provider);
    let tools = ToolRouter::default();
    let workspace = std::env::current_dir()?;
    let project_name = workspace
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into());

    let context_builder = ContextBuilder::new(&project_name);
    let agent = Agent::new(Box::from(provider), tools, context_builder);

    let mut state = AgentState {
        session_id: uuid::Uuid::new_v4(),
        task,
        messages: Vec::new(),
        plan: None,
        results: Vec::new(),
        working_memory: WorkingMemory::new(10_000),
        workspace,
        config: AgentConfig {
            model: config.model.clone(),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            max_tokens: 4096,
            max_iterations: 50,
            context_window: 128_000,
            temperature: 0.0,
            auto_compact: true,
            sandbox_policy: sigmacode_core::types::SandboxPolicy::DiskRead,
            mcp_servers: Vec::new(),
        },
        iteration: 0,
    };

    let cancel = tokio_util::sync::CancellationToken::new();
    let result = agent.run(&mut state, cancel, None).await?;

    println!("{}", result);

    Ok(())
}

struct AppConfig {
    provider: ProviderConfig,
    model: String,
    api_key: String,
    base_url: String,
}

fn load_config() -> anyhow::Result<AppConfig> {
    let api_key = std::env::var("SIGMACODE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default();

    let base_url = std::env::var("SIGMACODE_BASE_URL")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| "https://api.openai.com/v1".into());

    let model = std::env::var("SIGMACODE_MODEL")
        .or_else(|_| std::env::var("OPENAI_MODEL"))
        .unwrap_or_else(|_| "gpt-4o".into());

    let provider_type = std::env::var("SIGMACODE_PROVIDER").unwrap_or_else(|_| "openai".into());

    let provider = match provider_type.as_str() {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            ProviderConfig::Anthropic {
                api_key: key,
                model: model.clone(),
            }
        }
        "ollama" => ProviderConfig::Ollama {
            base_url: Some(base_url.clone()),
            model: model.clone(),
        },
        _ => ProviderConfig::OpenAi {
            api_key: api_key.clone(),
            base_url: Some(base_url.clone()),
            model: model.clone(),
        },
    };

    Ok(AppConfig {
        provider,
        model,
        api_key,
        base_url,
    })
}
