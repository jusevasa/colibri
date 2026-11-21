//! Sesiones de Claude Code: qué sesión corre en un proceso y cuáles siguen vivas en
//! segundo plano, para que al restaurar una pestaña se vuelva a *su* conversación.
//!
//! Desde la 2.1.330, Claude Code puede seguir corriendo una sesión en segundo plano
//! (control remoto, tareas aparcadas) aunque se cierre la terminal. Entonces
//! `claude --continue` no la abre: arranca una copia. Claude deja en
//! `~/.claude/sessions/<pid>.json` qué sesión lleva cada proceso (`sessionId`, `kind`,
//! `jobId` si va en segundo plano, `parkedJobId` si la interactiva aparcó una tarea); con
//! eso Colibrí sabe qué sesión era la de la pestaña y si sigue viva para `claude attach`.

use std::path::PathBuf;

/// Un proceso de Claude Code y la sesión que lleva.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub pid: i32,
    /// `sessionId`: la conversación.
    pub session: String,
    /// Corre en segundo plano (`kind = "bg"`): sobrevive a que se cierre la terminal.
    pub background: bool,
    /// En segundo plano, su `jobId` (lo que toma `claude attach`); en una interactiva, la
    /// tarea que aparcó en segundo plano (`parkedJobId`), si la hay.
    pub job: Option<String>,
}

/// Valor de `key` en un JSON plano y compacto como el de Claude: una cadena sin escapes o
/// un número. No es un lector de JSON general; para estos archivos basta.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let quoted = format!("\"{key}\"");
    let rest = &json[json.find(&quoted)? + quoted.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        Some(&s[..s.find('"')?])
    } else {
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '-'))
            .unwrap_or(rest.len());
        Some(&rest[..end])
    }
}

/// Lo que sale de aquí acaba en una orden que Enter ejecuta: solo letras, cifras y «-».
fn safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Lee un `~/.claude/sessions/<pid>.json`.
pub fn parse(json: &str) -> Option<Running> {
    let pid = field(json, "pid")?.parse().ok()?;
    let session = field(json, "sessionId").filter(|s| safe_id(s))?.to_string();
    let background = field(json, "kind") == Some("bg");
    let job = field(json, "jobId")
        .or_else(|| field(json, "parkedJobId"))
        .filter(|s| safe_id(s))
        .map(str::to_string);
    Some(Running {
        pid,
        session,
        background,
        job,
    })
}

/// `~/.claude/sessions`; `COLIBRI_CLAUDE_SESSIONS` la cambia para probar sin tocar la real.
fn sessions_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("COLIBRI_CLAUDE_SESSIONS") {
        return Some(PathBuf::from(p));
    }
    Some(dirs::home_dir()?.join(".claude/sessions"))
}

/// La sesión de Claude Code que corre en el proceso `pid`, si es uno.
pub fn of_pid(pid: i32) -> Option<Running> {
    let text = std::fs::read_to_string(sessions_dir()?.join(format!("{pid}.json"))).ok()?;
    parse(&text).filter(|r| r.pid == pid)
}

/// Sesiones de Claude Code cuyo proceso sigue vivo.
pub fn running() -> Vec<Running> {
    let Some(entries) = sessions_dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| parse(&std::fs::read_to_string(e.path()).ok()?))
        // kill(pid, 0) solo pregunta si el proceso existe.
        .filter(|r| unsafe { libc::kill(r.pid, 0) } == 0)
        .collect()
}

/// Orden para retomar la conversación de una pestaña donde corría Claude Code, sabiendo su
/// sesión y su tarea en segundo plano (vacías si no se supo) y las sesiones vivas:
/// si sigue viva en segundo plano, `claude attach` (abrirla de nuevo; `--continue` haría una
/// copia); si no, `claude --resume <sesión>` (esa, no la más reciente de la carpeta); sin
/// datos, `claude --continue` como antes.
pub fn resume_command(session: &str, job: &str, live: &[Running]) -> String {
    let attach = live.iter().filter(|r| r.background).find_map(|r| {
        let id = r.job.as_deref()?;
        (id == job || r.session == session).then_some(id)
    });
    match attach {
        Some(id) if !session.is_empty() || !job.is_empty() => format!("claude attach {id}"),
        _ if safe_id(session) => format!("claude --resume {session}"),
        _ => "claude --continue".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERACTIVA: &str = r#"{"pid":13492,"sessionId":"37d82006-4a61-4f7e-8a64-e0b92394a4cf","cwd":"/tmp","kind":"interactive","name":"colibri-e9","status":"idle","parkedJobId":"8de2fb68"}"#;
    const FONDO: &str = r#"{ "pid": 5095, "sessionId": "43d22477-5a1a-4c2a-80a8-dcfd3f588b8b", "kind": "bg", "jobId": "43d22477" }"#;

    #[test]
    fn lee_la_sesion_de_un_proceso() {
        let s = parse(INTERACTIVA).unwrap();
        assert_eq!(s.pid, 13492);
        assert_eq!(s.session, "37d82006-4a61-4f7e-8a64-e0b92394a4cf");
        assert!(!s.background);
        assert_eq!(s.job.as_deref(), Some("8de2fb68"));

        let s = parse(FONDO).unwrap();
        assert_eq!(s.pid, 5095);
        assert!(s.background);
        assert_eq!(s.job.as_deref(), Some("43d22477"));
    }

    #[test]
    fn rechaza_identificadores_raros() {
        // Lo que sale de aquí acaba en una orden que Enter ejecuta: solo letras, cifras y «-».
        assert!(parse(r#"{"pid":1,"sessionId":"abc; rm -rf ~","kind":"bg"}"#).is_none());
        let s = parse(r#"{"pid":1,"sessionId":"abc","kind":"bg","jobId":"$(x)"}"#).unwrap();
        assert_eq!(s.job, None);
        assert!(parse(r#"{"sessionId":"abc"}"#).is_none()); // sin pid
        assert!(parse("no es json").is_none());
    }

    fn fondo(session: &str, job: &str) -> Running {
        Running {
            pid: 1,
            session: session.into(),
            background: true,
            job: Some(job.into()),
        }
    }

    #[test]
    fn orden_para_retomar() {
        let vivas = [fondo("s-fondo", "8de2fb68")];
        // La tarea aparcada de la pestaña sigue viva: se abre esa.
        assert_eq!(
            resume_command("s1", "8de2fb68", &vivas),
            "claude attach 8de2fb68"
        );
        // La propia sesión sigue en segundo plano: también.
        assert_eq!(
            resume_command("s-fondo", "", &vivas),
            "claude attach 8de2fb68"
        );
        // Ya no corre: se retoma esa conversación, no «la más reciente de la carpeta».
        assert_eq!(resume_command("s1", "", &vivas), "claude --resume s1");
        assert_eq!(resume_command("s1", "otra", &[]), "claude --resume s1");
        // Sin saber cuál era (pestañas guardadas por versiones anteriores), como antes.
        assert_eq!(resume_command("", "", &vivas), "claude --continue");
    }
}
