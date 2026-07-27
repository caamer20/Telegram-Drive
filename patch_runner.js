const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(
    /pub trait ProcessRunner: Send \+ Sync \{[\s\S]*?ProcessOutput, String>> \+ Send \+ '_>>;[\s\S]*?\}/m,
    `pub trait ProcessRunner: Send + Sync {
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, String>> + Send + '_>>;
}`
);

code = code.replace(
    /impl ProcessRunner for RealProcessRunner \{[\s\S]*?Ok\(ProcessOutput \{[\s\S]*?\}\)[\s\S]*?\}\)[\s\S]*?\}/m,
    `impl ProcessRunner for RealProcessRunner {
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, String>> + Send + '_>> {
        let program = program.to_string();
        let args = args.to_vec();

        Box::pin(async move {
            let mut child = tokio::process::Command::new(&program)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("ProcessRunner: spawn failed: {}", e))?;

            let mut stdout = child.stdout.take().unwrap();
            let mut stderr = child.stderr.take().unwrap();

            let stdout_task = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0; 1024];
                let mut current_line = String::new();
                let mut full_output = Vec::new();
                while let Ok(n) = stdout.read(&mut buf).await {
                    if n == 0 { break; }
                    if full_output.len() < 1024 * 1024 {
                        full_output.extend_from_slice(&buf[..n]);
                    }
                    if let Some(ref cb) = on_progress {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for ch in text.chars() {
                            if ch == '\\n' || ch == '\\r' {
                                if !current_line.is_empty() {
                                    cb(&current_line);
                                    current_line.clear();
                                }
                            } else {
                                current_line.push(ch);
                            }
                        }
                    }
                }
                full_output
            });

            let stderr_task = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0; 4096];
                let mut full_output = Vec::new();
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 { break; }
                    if full_output.len() < 1024 * 1024 { // max 1MB
                        full_output.extend_from_slice(&buf[..n]);
                    }
                }
                full_output
            });

            let status = child.wait().await.map_err(|e| format!("Wait error: {}", e))?;
            let stdout_out = stdout_task.await.unwrap_or_default();
            let stderr_out = stderr_task.await.unwrap_or_default();

            Ok(ProcessOutput {
                exit_code: status.code().unwrap_or(-1),
                stdout: stdout_out,
                stderr: stderr_out,
            })
        })
    }
}`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
