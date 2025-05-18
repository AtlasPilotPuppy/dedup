use anyhow::Result;
use dedups::commands;
use dedups::options::Options;

fn main() -> Result<()> {
    // Load CLI args and config into Options
    let options = Options::new()?;
    
    // Run the application
    commands::run_app(&options)
}
