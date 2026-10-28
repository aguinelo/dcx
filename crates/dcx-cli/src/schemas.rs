use std::error::Error;
use std::fs;
use std::path::PathBuf;

use dcx_artifacts::json_schemas;

type SchemaResult<T> = Result<T, Box<dyn Error>>;

pub fn run(mut args: impl Iterator<Item = String>) -> SchemaResult<()> {
    let action = args.next().ok_or("expected --write or --check")?;
    let directory = args
        .next()
        .map_or_else(|| PathBuf::from("schemas"), PathBuf::from);
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let schemas = json_schemas()?;

    match action.as_str() {
        "--write" => {
            fs::create_dir_all(&directory)?;
            for (name, schema) in schemas {
                let path = directory.join(name);
                fs::write(&path, format!("{schema}\n"))?;
                println!("Gerado {}", path.display());
            }
        }
        "--check" => {
            for (name, schema) in schemas {
                let path = directory.join(name);
                let committed = fs::read_to_string(&path)?;
                if committed.trim_end() != schema {
                    return Err(format!(
                        "{} está desatualizado; rode dcx schemas --write",
                        path.display()
                    )
                    .into());
                }
                println!("OK {}", path.display());
            }
        }
        _ => return Err("expected --write or --check".into()),
    }
    Ok(())
}
