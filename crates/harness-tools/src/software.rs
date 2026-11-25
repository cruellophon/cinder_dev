use crate::sandbox::{resolve_workspace_path, strip_verbatim_prefix};
use crate::ToolExecutionResult;
use harness_providers::ToolSpec;
use harness_store::AllowedRuntimeEntry;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

pub fn runtime_presets() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        ("racket", "Racket", "racket"),
        ("python", "Python 3", "python"),
        ("node", "Node.js", "node"),
        ("cargo", "Rust / Cargo", "cargo"),
        ("go", "Go", "go"),
        ("gcc", "GCC", "gcc"),
        ("pdflatex", "pdflatex", "pdflatex"),
        ("typst", "Typst", "typst"),
        ("pandoc", "Pandoc", "pandoc"),
        ("ffmpeg", "FFmpeg", "ffmpeg"),
    ]
}

pub fn run_local_software_spec(allowed: &[AllowedRuntimeEntry]) -> Option<ToolSpec> {
    let enabled_ids: Vec<String> = allowed
        .iter()
        .filter(|r| r.enabled && !r.id.trim().is_empty() && !r.executable.trim().is_empty())
        .map(|r| r.id.trim().to_string())
        .collect();
    if enabled_ids.is_empty() {
        return None;
    }

    Some(ToolSpec {
        name: "run_local_software".into(),
        description: "Invoke an explicitly user-allowed local CLI runtime directly via OS argv without a shell (always pauses for user approval). Use this to compile/run code inside the active workspace or scratchpad (e.g. runtime_id='racket', args=['aufgaben.rkt']).".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "runtime_id": {
                    "type": "string",
                    "enum": enabled_ids,
                    "description": "ID of an enabled runtime from the user's Allowed CLI Runtimes list."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Direct OS argv list passed without a shell. Any file path must be relative to the workspace/scratchpad (no '..' or external paths)."
                }
            },
            "required": ["runtime_id", "args"]
        }),
    })
}

fn validate_runtime_path_arg(workspace_root: &Path, candidate: &str) -> anyhow::Result<()> {
    if candidate.starts_with('/') || candidate.starts_with('\\') || candidate.contains(':') {
        anyhow::bail!(
            "Absolute or drive-prefixed path '{candidate}' is blocked; pass a relative path inside the workspace/scratchpad."
        );
    }
    if Path::new(candidate)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("Path traversal ('..') is blocked in runtime argument: '{candidate}'");
    }
    let _ = resolve_workspace_path(workspace_root, candidate)?;
    Ok(())
}

fn looks_like_runtime_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.contains(':') || s.contains("..") || s.contains('.')
}

pub fn validate_runtime_args(workspace_root: &Path, args: &[String]) -> anyhow::Result<Vec<String>> {
    if args.len() > 16 {
        anyhow::bail!("Allowed runtime arguments exceed maximum limit (16 arguments).");
    }

    let mut validated = Vec::with_capacity(args.len());
    for raw in args {
        if raw.len() > 512 {
            anyhow::bail!("Runtime argument exceeds 512 characters.");
        }
        if raw.contains('\0') || raw.contains('\n') || raw.contains('\r') {
            anyhow::bail!("Runtime argument contains forbidden control characters.");
        }
        let trimmed = raw.trim();
        let lower = trimmed.to_ascii_lowercase();

        // Block inline code-eval / nested-command flags so execution always inspects visible workspace files.
        if matches!(
            lower.as_str(),
            "-e" | "--eval"
                | "-c"
                | "--command"
                | "-command"
                | "-encodedcommand"
                | "-enc"
                | "/c"
                | "/k"
                | "-p"
                | "--print"
                | "-qc"
                | "-uc"
                | "-ic"
                | "-sc"
                | "-ec"
                | "-pe"
                | "-qe"
                | "-ie"
                | "-te"
        ) || lower.starts_with("--eval=")
            || lower.starts_with("-e=")
            || lower.starts_with("-c=")
            || lower.starts_with("--command=")
            || lower.starts_with("--print=")
        {
            anyhow::bail!(
                "Inline eval/command flag '{trimmed}' is blocked in run_local_software; write code to a file inside the workspace/scratchpad and pass its filename instead."
            );
        }

        if trimmed.starts_with('-') {
            if let Some((_, rhs)) = trimmed.split_once('=') {
                let rhs_clean = rhs.trim();
                if !rhs_clean.is_empty() && looks_like_runtime_path(rhs_clean) {
                    validate_runtime_path_arg(workspace_root, rhs_clean)?;
                }
            }
        } else if looks_like_runtime_path(trimmed) {
            validate_runtime_path_arg(workspace_root, trimmed)?;
        }

        validated.push(raw.to_string());
    }
    Ok(validated)
}

fn is_blocked_batch_wrapper(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "bat" | "cmd" | "ps1"))
}

fn resolve_runtime_executable_path(raw: &str) -> anyhow::Result<PathBuf> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        anyhow::bail!("Configured runtime executable path is empty.");
    }
    let p = Path::new(trimmed);
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("Runtime executable path cannot contain '..'.");
    }
    if is_blocked_batch_wrapper(p) {
        anyhow::bail!(
            "Batch/script wrappers (.bat/.cmd/.ps1) are blocked in Allowed CLI Runtimes; configure the direct binary executable instead: '{trimmed}'"
        );
    }

    if trimmed.contains('/') || trimmed.contains('\\') {
        if !p.is_file() {
            anyhow::bail!("Configured runtime executable does not exist: '{trimmed}'");
        }
        return Ok(p.to_path_buf());
    }

    // Resolve bare binary name against PATH so child spawns with minimal env_clear() PATH.
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(trimmed);
        if direct.is_file() && !is_blocked_batch_wrapper(&direct) {
            return Ok(direct);
        }
        #[cfg(windows)]
        if p.extension().is_none() {
            for ext in ["exe", "com"] {
                let with_ext = dir.join(format!("{trimmed}.{ext}"));
                if with_ext.is_file() {
                    return Ok(with_ext);
                }
            }
        }
    }

    anyhow::bail!("Could not find configured runtime '{trimmed}' on PATH.")
}

fn configure_minimal_runtime_env(cmd: &mut tokio::process::Command, exe_path: &Path, tmp_dir: &Path) {
    cmd.env_clear();

    let exe_parent = exe_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| strip_verbatim_prefix(p.to_path_buf()));

    #[cfg(windows)]
    {
        for k in [
            "SYSTEMROOT",
            "WINDIR",
            "PROCESSOR_ARCHITECTURE",
            "NUMBER_OF_PROCESSORS",
        ] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        let sys_root = std::env::var_os("SYSTEMROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let mut path_parts: Vec<PathBuf> = Vec::new();
        if let Some(ref ep) = exe_parent {
            path_parts.push(ep.clone());
        }
        path_parts.push(sys_root.join("System32"));
        path_parts.push(sys_root);
        if let Ok(joined) = std::env::join_paths(path_parts) {
            cmd.env("PATH", joined);
        }
        cmd.env("TEMP", tmp_dir);
        cmd.env("TMP", tmp_dir);
        cmd.env("TMPDIR", tmp_dir);
        cmd.env("HOME", tmp_dir);
        cmd.env("USERPROFILE", tmp_dir);
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("NO_COLOR", "1");
    }

    #[cfg(not(windows))]
    {
        let mut path_parts: Vec<PathBuf> = Vec::new();
        if let Some(ref ep) = exe_parent {
            path_parts.push(ep.clone());
        }
        for std_dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            let p = PathBuf::from(std_dir);
            if p.is_dir() {
                path_parts.push(p);
            }
        }
        if let Ok(joined) = std::env::join_paths(path_parts) {
            cmd.env("PATH", joined);
        }
        cmd.env("HOME", tmp_dir);
        cmd.env("TMPDIR", tmp_dir);
        cmd.env("LANG", "C.UTF-8");
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("NO_COLOR", "1");
    }

    let exe_stem = exe_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(exe_stem.as_str(), "cargo" | "rustc" | "rustup") {
        if let Some(v) = std::env::var_os("RUSTUP_TOOLCHAIN") {
            cmd.env("RUSTUP_TOOLCHAIN", v);
        }
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".rustup")))
            .filter(|p| p.is_dir());
        if let Some(rh) = rustup_home {
            cmd.env("RUSTUP_HOME", rh);
        }
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
            .filter(|p| p.is_dir());
        if let Some(ch) = cargo_home {
            cmd.env("CARGO_HOME", ch);
        }
    }
}

#[cfg(windows)]
mod win_job {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr;

    const JOB_OBJECT_EXTENDED_LIMIT_INFO: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const PROCESS_ALL_ACCESS: u32 = 0x001F_0FFF;

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *const c_void, lp_name: *const u16) -> isize;
        fn SetInformationJobObject(
            h_job: isize,
            job_object_info_class: i32,
            lp_job_object_info: *const c_void,
            cb_job_object_info_length: u32,
        ) -> i32;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> isize;
        fn AssignProcessToJobObject(h_job: isize, h_process: isize) -> i32;
        fn CloseHandle(h_object: isize) -> i32;
    }

    pub struct WinJobGuard(isize);

    impl WinJobGuard {
        pub fn attach_pid(pid: u32) -> Option<Self> {
            unsafe {
                let job = CreateJobObjectW(ptr::null(), ptr::null());
                if job == 0 {
                    return None;
                }
                let mut info: JobObjectExtendedLimitInformation = zeroed();
                info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok_info = SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFO,
                    (&info as *const JobObjectExtendedLimitInformation) as *const c_void,
                    size_of::<JobObjectExtendedLimitInformation>() as u32,
                );
                if ok_info == 0 {
                    CloseHandle(job);
                    return None;
                }
                let proc_h = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
                if proc_h == 0 {
                    CloseHandle(job);
                    return None;
                }
                let assigned = AssignProcessToJobObject(job, proc_h);
                CloseHandle(proc_h);
                if assigned == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(Self(job))
            }
        }
    }

    impl Drop for WinJobGuard {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }
}

pub async fn execute_allowed_runtime(
    workspace_root: &Path,
    allowed: &[AllowedRuntimeEntry],
    args: &Value,
) -> anyhow::Result<ToolExecutionResult> {
    if workspace_root.as_os_str().is_empty() || !workspace_root.is_dir() {
        anyhow::bail!("No valid workspace/scratchpad directory is attached.");
    }
    let cwd = strip_verbatim_prefix(
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf()),
    );

    let runtime_id = args["runtime_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing 'runtime_id' argument."))?;

    let entry = allowed
        .iter()
        .find(|r| r.enabled && r.id == runtime_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Runtime '{runtime_id}' is not enabled in Settings → Allowed CLI Runtimes."
            )
        })?;

    let arg_list: Vec<String> = match args.get("args") {
        None => Vec::new(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("Each element of 'args' must be a string."))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("'args' must be an array of strings."),
    };

    let validated_args = validate_runtime_args(&cwd, &arg_list)?;
    let exe_path = resolve_runtime_executable_path(&entry.executable)?;

    let tmp_dir = cwd.join(".cinder_tmp");
    std::fs::create_dir_all(&tmp_dir)?;

    let mut cmd = tokio::process::Command::new(&exe_path);
    cmd.args(&validated_args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_minimal_runtime_env(&mut cmd, &exe_path, &tmp_dir);

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Failed to launch '{}' ({}): {e}",
            entry.label,
            exe_path.display()
        )
    })?;

    #[cfg(windows)]
    let _job_guard = child.id().and_then(win_job::WinJobGuard::attach_pid);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>)>();
    if let Some(mut stdout_pipe) = child.stdout.take() {
        let tx_out = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok(n) = stdout_pipe.read(&mut buf).await {
                if n == 0 || tx_out.send((false, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(mut stderr_pipe) = child.stderr.take() {
        let tx_err = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok(n) = stderr_pipe.read(&mut buf).await {
                if n == 0 || tx_err.send((true, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut stdout_bytes: Vec<u8> = Vec::new();
    let mut stderr_bytes: Vec<u8> = Vec::new();
    let run_res = tokio::time::timeout(Duration::from_secs(25), async {
        while let Some((is_err, chunk)) = rx.recv().await {
            if is_err {
                if stderr_bytes.len() < 16_384 {
                    stderr_bytes.extend_from_slice(&chunk);
                }
            } else if stdout_bytes.len() < 16_384 {
                stdout_bytes.extend_from_slice(&chunk);
            }
        }
        child.wait().await
    })
    .await;

    let status = match run_res {
        Ok(res) => res?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("Runtime '{}' timed out after 25s and its process tree was terminated.", entry.label);
        }
    };

    let code = status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    let mut combined = format!(
        "Runtime: {} ({})\nExit code: {code}\n",
        entry.label,
        exe_path.display()
    );
    if !stdout.trim().is_empty() {
        combined.push_str("STDOUT:\n");
        combined.push_str(&stdout);
        combined.push('\n');
    }
    if !stderr.trim().is_empty() {
        combined.push_str("STDERR:\n");
        combined.push_str(&stderr);
        combined.push('\n');
    }
    if combined.len() > 16_000 {
        let mut end = 16_000;
        while !combined.is_char_boundary(end) {
            end -= 1;
        }
        combined.truncate(end);
        combined.push_str("\n... [output truncated]");
    }

    Ok(ToolExecutionResult {
        output: combined,
        diff: None,
        file_path: None,
        is_error: !status.success(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_local_software_spec_is_none_when_no_enabled_runtimes() {
        assert!(run_local_software_spec(&[]).is_none());
        let disabled = vec![AllowedRuntimeEntry {
            id: "racket".into(),
            label: "Racket".into(),
            executable: "racket".into(),
            enabled: false,
        }];
        assert!(run_local_software_spec(&disabled).is_none());

        let enabled = vec![AllowedRuntimeEntry {
            id: "racket".into(),
            label: "Racket".into(),
            executable: "racket".into(),
            enabled: true,
        }];
        let spec = run_local_software_spec(&enabled).unwrap();
        assert_eq!(spec.name, "run_local_software");
    }

    #[test]
    fn validate_runtime_args_blocks_eval_flags_traversal_and_newlines() {
        let dir = std::env::temp_dir().join(format!("cinder_rt_args_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aufgaben.rkt"), "#lang racket\n(+ 1 2)\n").unwrap();

        assert!(validate_runtime_args(&dir, &["aufgaben.rkt".into()]).is_ok());

        for bad in [
            vec!["-e".into(), "(system \"calc\")".into()],
            vec!["-c".into(), "import os".into()],
            vec!["-qc".into(), "import os".into()],
            vec!["-p".into(), "process.exit(1)".into()],
            vec!["--output=C:\\Windows\\win.ini".into()],
            vec!["--file=../outside.txt".into()],
            vec!["../outside.rkt".into()],
            vec!["C:\\Windows\\win.ini".into()],
            vec!["foo\nbar".into()],
        ] {
            assert!(
                validate_runtime_args(&dir, &bad).is_err(),
                "expected block for {:?}",
                bad
            );
        }

        assert!(resolve_runtime_executable_path("wrapper.bat").is_err());
        assert!(resolve_runtime_executable_path("wrapper.cmd").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn execute_allowed_runtime_runs_with_scrubbed_env_and_job_containment() {
        let dir = std::env::temp_dir().join(format!("cinder_rt_exec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let allowed = vec![AllowedRuntimeEntry {
            id: "cargo".into(),
            label: "Rust / Cargo".into(),
            executable: "cargo".into(),
            enabled: true,
        }];
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let res = execute_allowed_runtime(
                &dir,
                &allowed,
                &json!({ "runtime_id": "cargo", "args": ["--version"] }),
            )
            .await
            .unwrap();
            assert!(!res.is_error, "unexpected error: {}", res.output);
            assert!(res.output.contains("cargo "), "output: {}", res.output);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
