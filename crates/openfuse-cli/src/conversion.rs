use openfuse_core::{
    conversion::{Options, Plan},
    preset_file,
};
use std::{ffi::OsString, path::PathBuf};

struct Arguments {
    source: PathBuf,
    output: Option<PathBuf>,
    options: Options,
    accepted: bool,
}
fn arguments(args: Vec<OsString>, write: bool) -> Result<Arguments, String> {
    let mut source = None;
    let mut target = None;
    let mut output = None;
    let mut repair = false;
    let mut normalize = false;
    let mut accepted = false;
    let mut assume_missing_gdec = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let key = arg.to_str().ok_or("Invalid option")?;
        if !seen.insert(key.to_owned()) {
            return Err(format!("Duplicate option {key}"));
        }
        match key {
            "--file" => source = Some(PathBuf::from(args.next().ok_or("Missing --file value")?)),
            "--target" => {
                target = Some(
                    args.next()
                        .ok_or("Missing --target value")?
                        .to_str()
                        .ok_or("Invalid target")?
                        .parse::<u32>()
                        .map_err(|_| "Target must be a decimal preset ProductId")?,
                )
            }
            "--out" if write => {
                output = Some(PathBuf::from(args.next().ok_or("Missing --out value")?))
            }
            "--repair-double-entities" => repair = true,
            "--normalize-legacy-delays" => normalize = true,
            "--assume-missing-gdec" => assume_missing_gdec = true,
            "--accept-changes" if write => accepted = true,
            _ => return Err(format!("Unknown option {key}")),
        }
    }
    if write && (!accepted || output.is_none()) {
        return Err("Writing a converted copy requires --out NEW_FILE and --accept-changes; inspect preset-convert-plan first".into());
    }
    Ok(Arguments {
        source: source.ok_or("Missing --file")?,
        output,
        accepted,
        options: Options {
            target_product: target.ok_or("Missing --target")?,
            repair_double_entities: repair,
            normalize_legacy_delays: normalize,
            assume_missing_gdec,
        },
    })
}
pub fn run(args: Vec<OsString>, write: bool) -> Result<(), String> {
    let args = arguments(args, write)?;
    let original = preset_file::read(&args.source).map_err(|e| e.to_string())?;
    let plan = Plan::preview(&original, args.options).map_err(|e| e.to_string())?;
    let mut output = serde_json::json!({
        "kind":"offline_preset_conversion", "transmitted":false,"options":args.options,
        "source_product":plan.source_product(),"target_product":plan.target_product(),
        "source_fingerprint":plan.source_fingerprint(),"changes":plan.changes(),
        "xml":plan.result().to_xml().map_err(|e|e.to_string())?,
        "written":false,
    });
    if let Some(path) = args.output {
        assert!(args.accepted);
        // Revalidate the source after preparation, before publishing a new copy.
        let current = preset_file::read(&args.source).map_err(|e| e.to_string())?;
        let converted = plan.apply(&current).map_err(|e| e.to_string())?;
        preset_file::write_new(&converted, &path).map_err(|e| e.to_string())?;
        output["written"] = true.into();
        output["output"] = serde_json::to_value(&path).map_err(|e| e.to_string())?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}
