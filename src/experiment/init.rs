use crate::*;

use super::util::{
    copy_fixture_workdir, experiment_dir, experiment_readme, mixmod_notes_template,
    placeholder_experiment_metrics, seed_experiment_task_from_fixture, task_json_template,
    task_md_template, validate_experiment_name,
};

pub fn experiment_init(root: &Path, name: &str, fixture: Option<&Path>) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = experiment_dir(root, name);
    let mixmod_dir = exp_dir.join("mixmod");
    let work_dir = exp_dir.join("work");
    for dir in [&mixmod_dir, &mixmod_dir.join("runs"), &work_dir] {
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }

    write_if_missing(&exp_dir.join(TASK_MD), task_md_template(name).as_bytes())?;
    write_pretty_json_if_missing(
        &exp_dir.join(TASK_JSON),
        &task_json_template(name),
        "experiment task template",
    )?;
    write_if_missing(
        &exp_dir.join("README.md"),
        experiment_readme(name).as_bytes(),
    )?;
    write_if_missing(
        &mixmod_dir.join("notes.md"),
        mixmod_notes_template(name).as_bytes(),
    )?;
    write_if_missing(&mixmod_dir.join(FINAL_PATCH), b"")?;
    write_pretty_json_if_missing(
        &mixmod_dir.join(METRICS_JSON),
        &placeholder_experiment_metrics("codex-plus-mixmod"),
        "mixmod placeholder metrics",
    )?;
    if let Some(fixture) = fixture {
        let fixture_path = absolutize(root, fixture);
        seed_experiment_task_from_fixture(&fixture_path, &exp_dir, root)?;
        copy_fixture_workdir(&fixture_path, &work_dir.join("mixmod"), "mixmod", root)?;
        copy_fixture_workdir(&fixture_path, &work_dir.join("default"), "default", root)?;
    }

    println!(
        "created experiment scaffold at {}",
        display_path(root, &exp_dir)
    );
    println!("task templates:");
    println!("  {}", display_path(root, &exp_dir.join(TASK_MD)));
    println!("  {}", display_path(root, &exp_dir.join(TASK_JSON)));
    if fixture.is_some() {
        println!("isolated work dirs:");
        println!("  {}", display_path(root, &work_dir.join("mixmod")));
        println!("  {}", display_path(root, &work_dir.join("default")));
    }
    Ok(())
}
