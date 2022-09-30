#[derive(Debug, clap::Parser)]
pub struct Cli {
    /**
     * Dirs to look for fence and source files
     * Evaluated relative to the `--project` file
     */
    pub paths: Vec<String>,

    /**
     * The tsconfig file used relative to '--root' argument
     */
    #[clap(short, long, default_value = "tsconfig.json")]
    pub project: String,

    /**
     *  Overrides `compilerOptions.baseUrl` property read from '--project' argument
     */
    #[clap(short, long)]
    pub base_url: Option<String>,

    /**
     * Output file for violations, relative to '--root' argument
     */
    #[clap(short, long, default_value = "good-fences-violations.json")]
    pub output: String,
}
