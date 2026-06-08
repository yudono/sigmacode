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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    let task = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        eprintln!("Usage: sigmacode-headless <task>");
        eprintln!("Example: sigmacode-headless \"Add Google OAuth to the app\"");
        std::process::exit(1);
    };

    let (provider_config, model, api_key, base_url, config_workspace) = load_sigma_config()?;

    eprintln!("SigmaCode (headless) - Task: {}", task);

    let provider = create_provider(&provider_config);
    let tools = ToolRouter::default();
    let workspace = config_workspace.as_ref()
        .and_then(|w| {
            let p = std::path::PathBuf::from(w);
            if p.exists() { Some(p) } else { None }
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project_name = workspace
        .file_name()
        .map(|n: &std::ffi::OsStr| n.to_string_lossy().to_string())
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
            model,
            api_key,
            base_url: base_url.unwrap_or_default(),
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

#[derive(serde::Deserialize)]
struct SigmaConfig {
    provider: String,
    model: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

fn load_sigma_config() -> anyhow::Result<(ProviderConfig, String, String, Option<String>, Option<String>)> {
    let config_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".sigma")
        .join("config.yml");

    if !config_path.exists() {
        anyhow::bail!(
            "Config not found at ~/.sigma/config.yml\nRun `sigmacode` (TUI) first to complete setup."
        );
    }

    let content = std::fs::read_to_string(&config_path)?;
    let cfg: SigmaConfig = serde_yaml::from_str(&content)?;

    let provider = match cfg.provider.as_str() {
        "anthropic" => ProviderConfig::Anthropic {
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        },
        "ollama" => ProviderConfig::Ollama {
            base_url: Some(
                cfg.base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".into()),
            ),
            model: cfg.model.clone(),
        },
        "gemini" => ProviderConfig::Gemini {
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        },
        _ => ProviderConfig::OpenAi {
            api_key: cfg.api_key.clone(),
            base_url: Some(
                cfg.base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".into()),
            ),
            model: cfg.model.clone(),
        },
    };

    Ok((provider, cfg.model, cfg.api_key, cfg.base_url, cfg.workspace))
}
