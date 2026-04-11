//! Makima - A local coding assistant powered by LM Studio
//!
//! Run with `cargo run` for interactive CLI mode, or `cargo run -- serve` for web interface.

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::*;
use makima::{
    cli::Repl,
    config::{Config, ToolSet},
    llm::LmStudioClient,
    web::run_server,
};
use std::io::{self, Write};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "makima")]
#[command(author = "Makima Team")]
#[command(version = "0.1.1")]
#[command(about = "A local coding assistant powered by LM Studio", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<String>,

    /// LM Studio API URL
    #[arg(long, global = true)]
    url: Option<String>,

    /// Espace de travail (répertoire par défaut pour les opérations)
    #[arg(short = 'e', long = "espace-de-travail", alias = "workdir", global = true)]
    espace_de_travail: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive CLI mode (default)
    Chat {
        /// Optional initial message
        message: Option<String>,
    },

    /// Start the web server
    Serve {
        /// Port to listen on
        #[arg(short, long)]
        port: Option<u16>,

        /// Host address
        #[arg(long)]
        host: Option<String>,
    },

    /// Check connection to LM Studio
    Status,

    /// Initialize configuration file
    Init {
        /// Force overwrite existing config
        #[arg(short, long)]
        force: bool,
    },
}

/// Pre-launch checks: wait for LM Studio and choose max_tokens
async fn pre_launch_checks(config: &mut Config) -> Result<()> {
    // 1. Wait for LM Studio
    println!("Verification de LM Studio...");
    let client = LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model);
    loop {
        match client.health_check().await {
            Ok(true) => {
                println!("{} LM Studio connecte", "✓".green());
                if let Ok(models) = client.list_models().await {
                    for m in &models {
                        println!("  Modele: {}", m);
                    }
                }
                break;
            }
            _ => {
                println!("{} En attente de LM Studio ({})...", "⏳", config.lm_studio.url);
                println!("   Lancez LM Studio et chargez un modele.");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }

    // 2. Choose max_tokens
    println!();
    println!("Fenetre de contexte (max tokens) :");
    println!("  {} 4096   - Rapide, echanges courts", "[1]".cyan());
    println!("  {} 8192   - Conversations longues", "[2]".cyan());
    println!("  {} 32768  - Documents volumineux", "[3]".cyan());
    println!("  {} 131072 - Contexte maximum (131K)", "[4]".cyan());
    println!();
    println!("  {} Plus la valeur est elevee, plus le modele peut traiter de texte", "ℹ".blue());
    println!("    en une fois, mais les reponses seront plus lentes.");
    print!("{} Choix [1/2/3/4, defaut=1]: ", "▶".green());
    io::stdout().flush()?;

    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;

    match choice.trim() {
        "2" => {
            config.lm_studio.max_tokens = 8192;
            println!("{} Max tokens: 8192", "✓".green());
        }
        "3" => {
            config.lm_studio.max_tokens = 32768;
            println!("{} Max tokens: 32768", "✓".green());
        }
        "4" => {
            config.lm_studio.max_tokens = 131072;
            println!("{} Max tokens: 131072 (131K)", "✓".green());
        }
        _ => {
            config.lm_studio.max_tokens = 4096;
            println!("{} Max tokens: 4096", "✓".green());
        }
    }
    println!();

    // 3. Choose tool set
    let current = match config.tools.tool_set {
        ToolSet::Standard => "Standard",
        ToolSet::Akari => "Akari",
    };
    println!("Jeu d'outils (actuel: {}):", current);
    println!("  {} Outils standards  - Les outils actuels de Makima", "[1]".cyan());
    println!("  {} Outils Akari ({}) - Optimises pour GLM-4.6V, inspires de Claude Code",
        "[2]".cyan(), "灯".yellow());
    println!();
    println!("  {} Les outils Akari incluent web_fetch, web_search, et des schemas", "ℹ".blue());
    println!("    ameliores pour le function calling natif de GLM-4.6V.");
    print!("{} Choix [1/2, defaut=actuel]: ", "▶".green());
    io::stdout().flush()?;

    let mut tool_choice = String::new();
    io::stdin().read_line(&mut tool_choice)?;

    match tool_choice.trim() {
        "1" => {
            config.tools.tool_set = ToolSet::Standard;
            println!("{} Jeu d'outils: Standard", "✓".green());
        }
        "2" => {
            config.tools.tool_set = ToolSet::Akari;
            println!("{} Jeu d'outils: Akari (灯)", "✓".green());
        }
        _ => {
            println!("{} Jeu d'outils: {} (inchange)", "✓".green(), current);
        }
    }
    println!();

    Ok(())
}

/// Check if this is the first run and prompt for initial configuration
async fn check_first_run_setup(config: &mut Config) -> Result<bool> {
    let config_path = dirs::home_dir()
        .map(|h| h.join(".makima").join("config.toml"));

    let is_first_run = config_path.as_ref().map(|p| !p.exists()).unwrap_or(true);
    let needs_workdir = config.tools.working_dir.is_empty();

    // If config exists and workdir is set, no setup needed
    if !is_first_run && !needs_workdir {
        return Ok(false);
    }

    // Show setup prompt
    if is_first_run {
        println!();
        println!("{}", "╔══════════════════════════════════════════╗".cyan());
        println!("{}", "║      Bienvenue dans Makima !             ║".cyan().bold());
        println!("{}", "║      Configuration initiale              ║".cyan());
        println!("{}", "╚══════════════════════════════════════════╝".cyan());
        println!();
    }

    // Always ask for working directory if empty
    if needs_workdir {
        let current_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        println!("{} Repertoire de travail", "?".cyan().bold());
        println!("  {} Repertoire courant: {}", "1.".dimmed(), current_dir.green());
        println!("  {} Personnalise (saisir le chemin)", "2.".dimmed());
        println!("  {} Toujours demander", "3.".dimmed());
        print!("{} Choix [1/2/3, defaut=1]: ", "▶".green());
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        match choice.trim() {
            "2" => {
                print!("{} Chemin du repertoire: ", "▶".green());
                io::stdout().flush()?;

                let mut custom_path = String::new();
                io::stdin().read_line(&mut custom_path)?;
                let custom_path = custom_path.trim().to_string();

                if !custom_path.is_empty() && std::path::Path::new(&custom_path).exists() {
                    config.tools.working_dir = custom_path.clone();
                    println!("{} Repertoire: {}", "✓".green(), custom_path);
                } else if !custom_path.is_empty() {
                    println!("{} Le repertoire n'existe pas, utilisation du repertoire courant", "⚠".yellow());
                    config.tools.working_dir = current_dir;
                } else {
                    config.tools.working_dir = current_dir;
                }
            }
            "3" => {
                // Leave empty - will use current dir each time
                println!("{} Le repertoire courant sera utilise a chaque lancement", "ℹ".blue());
            }
            _ => {
                // Default: use current directory
                config.tools.working_dir = current_dir.clone();
                println!("{} Repertoire: {}", "✓".green(), current_dir);
            }
        }

        println!();
    }

    // Save config if first run
    if is_first_run {
        print!("{} Sauvegarder cette configuration? [O/n]: ", "?".cyan());
        io::stdout().flush()?;

        let mut save_choice = String::new();
        io::stdin().read_line(&mut save_choice)?;

        if !save_choice.trim().eq_ignore_ascii_case("n") {
            let path = config.save_default().await?;
            println!("{} Configuration sauvegardee: {}", "✓".green(), path.display());
        }
        println!();
    }

    Ok(true)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();

    // Load configuration
    let mut config = if let Some(ref path) = cli.config {
        Config::load(path).await?
    } else {
        Config::load_default().await.unwrap_or_default()
    };

    // Apply CLI overrides
    if let Some(ref url) = cli.url {
        config.lm_studio.url = url.clone();
    }

    // Si --espace-de-travail est fourni, l'utiliser
    // Sinon, garder le working_dir de la config (defini au premier lancement)
    if let Some(ref espace) = cli.espace_de_travail {
        config.tools.working_dir = espace.clone();
    }

    // Check for first run setup (only for interactive modes)
    match &cli.command {
        None | Some(Commands::Chat { .. }) => {
            check_first_run_setup(&mut config).await?;
        }
        _ => {}
    }

    // Pre-launch checks for interactive modes (not Status/Init)
    match &cli.command {
        Some(Commands::Status) | Some(Commands::Init { .. }) => {}
        _ => {
            pre_launch_checks(&mut config).await?;
        }
    }

    match cli.command {
        Some(Commands::Chat { message }) => {
            // Lancer le serveur web en arrière-plan
            let server_config = config.clone();
            tokio::spawn(async move {
                if let Err(e) = run_server(server_config).await {
                    eprintln!("Erreur serveur web: {}", e);
                }
            });

            let mut repl = Repl::new(&config).await?;

            if let Some(msg) = message {
                // Single message mode
                println!("{} {}", "You:".green().bold(), msg);
                // Process message and exit
                // For now, just run the REPL
            }

            repl.run().await?;
        }

        Some(Commands::Serve { port, host }) => {
            if let Some(p) = port {
                config.server.port = p;
            }
            if let Some(h) = host {
                config.server.host = h;
            }

            run_server(config).await?;
        }

        Some(Commands::Status) => {
            println!("{}", "Checking LM Studio connection...".cyan());
            println!("URL: {}", config.lm_studio.url);

            let client = LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model);

            match client.health_check().await {
                Ok(true) => {
                    println!("{} LM Studio is running", "✓".green());

                    match client.list_models().await {
                        Ok(models) => {
                            println!("\nAvailable models:");
                            for model in models {
                                println!("  - {}", model);
                            }
                        }
                        Err(e) => {
                            println!("{} Could not list models: {}", "⚠".yellow(), e);
                        }
                    }
                }
                Ok(false) | Err(_) => {
                    println!("{} LM Studio is not responding", "✗".red());
                    println!("\nMake sure:");
                    println!("  1. LM Studio is running");
                    println!("  2. A model is loaded");
                    println!("  3. The local server is enabled (Developer → Local Server)");
                    println!("  4. The URL is correct: {}", config.lm_studio.url);
                }
            }
        }

        Some(Commands::Init { force }) => {
            let config_path = dirs::home_dir()
                .map(|h| h.join(".makima").join("config.toml"))
                .unwrap_or_else(|| std::path::PathBuf::from("config.toml"));

            if config_path.exists() && !force {
                println!(
                    "{} Config file already exists at {}",
                    "⚠".yellow(),
                    config_path.display()
                );
                println!("Use --force to overwrite");
                return Ok(());
            }

            let mut new_config = Config::default();

            // Ask for working directory
            println!("{}", "Configuration de Makima".cyan().bold());
            println!();

            let current_dir = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string());

            println!("{} Repertoire de travail", "?".cyan().bold());
            println!("  {} Repertoire courant: {}", "1.".dimmed(), current_dir.green());
            println!("  {} Personnalise (saisir le chemin)", "2.".dimmed());
            println!("  {} Vide (demander a chaque lancement)", "3.".dimmed());
            print!("{} Choix [1/2/3]: ", "▶".green());
            std::io::Write::flush(&mut std::io::stdout())?;

            let mut choice = String::new();
            std::io::stdin().read_line(&mut choice)?;

            match choice.trim() {
                "1" => {
                    new_config.tools.working_dir = current_dir.clone();
                    println!("{} Repertoire: {}", "✓".green(), current_dir);
                }
                "2" => {
                    print!("{} Chemin du repertoire: ", "▶".green());
                    std::io::Write::flush(&mut std::io::stdout())?;

                    let mut custom_path = String::new();
                    std::io::stdin().read_line(&mut custom_path)?;
                    let custom_path = custom_path.trim().to_string();

                    if std::path::Path::new(&custom_path).exists() {
                        new_config.tools.working_dir = custom_path.clone();
                        println!("{} Repertoire: {}", "✓".green(), custom_path);
                    } else {
                        println!("{} Le repertoire n'existe pas, utilisation du repertoire courant", "⚠".yellow());
                        new_config.tools.working_dir = current_dir;
                    }
                }
                "3" | "" => {
                    new_config.tools.working_dir = String::new();
                    println!("{} Repertoire sera demande a chaque lancement", "ℹ".blue());
                }
                _ => {
                    new_config.tools.working_dir = current_dir.clone();
                    println!("{} Choix invalide, utilisation du repertoire courant: {}", "⚠".yellow(), current_dir);
                }
            }

            println!();

            // Ask for LM Studio URL
            println!("{} URL de LM Studio", "?".cyan().bold());
            println!("  Par defaut: {}", "http://localhost:1234/v1".dimmed());
            print!("{} URL (Entree pour defaut): ", "▶".green());
            std::io::Write::flush(&mut std::io::stdout())?;

            let mut url_input = String::new();
            std::io::stdin().read_line(&mut url_input)?;
            let url_input = url_input.trim();

            if !url_input.is_empty() {
                new_config.lm_studio.url = url_input.to_string();
                println!("{} URL: {}", "✓".green(), url_input);
            } else {
                println!("{} URL: {}", "✓".green(), new_config.lm_studio.url);
            }

            println!();

            let path = new_config.save_default().await?;
            println!("{} Configuration sauvegardee: {}", "✓".green(), path.display());
        }

        None => {
            // Lancer le serveur web en arrière-plan
            let server_config = config.clone();
            tokio::spawn(async move {
                if let Err(e) = run_server(server_config).await {
                    eprintln!("Erreur serveur web: {}", e);
                }
            });

            // Default to chat mode
            let mut repl = Repl::new(&config).await?;
            repl.run().await?;
        }
    }

    Ok(())
}
