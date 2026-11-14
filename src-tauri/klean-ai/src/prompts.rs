//! Every text KLEAN puts in front of a model, in one place.
//!
//! The system prompt, the description of each tool, its parameters, and the
//! sentence that introduces the reduced Lean context: all of it lives here, as
//! plain constants, so it can be read and changed without touching the rest of the
//! code. Nothing else in the crate writes prose for a model.

/// The opening message of every session.
pub const SYSTEM: &str = "This is a Lean4 project, your goal is to assist the user with it. \n\n\
- Before every prompt you'll be given a working Lean4 file with a subset of terms from the main project, called LCONTEXT. \
- If you need a bigger subset, tool `view_ast` allows you to look around, tool `expand` adds \
other nodes to LCONTEXT (dependee nodes are also added, no need to ask for them), and tool `focus` \
replaces LCONTEXT with only the nodes you name. \
- Tool `modify_ast` adds, edits or removes declarations of the project. Its changes are written to \
the project's files, so make them only when the user asked for a change. \
- Answer in Markdown. A diagram helps more often than prose: write it as a mermaid fence \
(```mermaid) in your answer and it is drawn next to what you wrote. \
- Don't assume anything beyond what's deducible from LCONTEXT, there's no limit for the `ask` and `ask_multi` tools. \
- You MUST only answer based on the axioms of LCONTEXT, nothing else. Short and concise answers. \
- Non-trivial answers require using the tool `lean_check` to validate hypothesis. \
- When presenting evidence, always include the filename and line; a code snippet; and the result of #check for an example (if non-trivial).";

/// The line that introduces the user's own instructions, when they added any in
/// `Settings → AI → Customize`.
pub const CUSTOM_INTRO: &str = "Additional steering (doesn't override the above):";

/// The line that introduces the reduced Lean source in the conversation.
pub const CONTEXT_INTRO: &str = "LCONTEXT:";

/// What `expand` says back when it brought nodes in.
pub const EXPAND_INTRO: &str = "LCONTEXT expanded with new nodes:";

/// What `focus` says back when it replaced the whole context.
pub const FOCUS_INTRO: &str = "LCONTEXT replaced with these nodes:";

/// The line a session that was started for one declaration puts before the context
/// — every time the context is sent, not only at the start, so a long session does
/// not drift away from the term it was opened for.
pub fn target_intro(name: &str) -> String {
    format!("Focus on term {name}. Use other terms as context.")
}

// ------------------------------------------------------- tool results (text the model reads)

/// A tool name the model invented.
pub fn no_such_tool(name: &str) -> String {
    format!("There is no tool called `{name}`. Use one of the tools you were given.")
}

/// A tool that failed, reported as its result so the model can recover.
pub fn tool_failed(reason: &str) -> String {
    format!("The tool failed: {reason}")
}

/// A tool KLEAN knows but has no implementation for.
pub fn not_implemented(name: &str) -> String {
    format!("The tool `{name}` is not implemented")
}

/// The result of `lean_check` when Lean had nothing to say about the term.
pub fn check_silent(expression: &str) -> String {
    format!("Lean accepted `{expression}` and printed nothing.")
}

/// The result of `lean_check` when Lean could not run at all (no build output, a
/// timeout, a missing toolchain).
pub fn check_failed(reason: &str) -> String {
    format!("`lean_check` could not run: {reason}")
}

/// The result of `expand`: what changed, and the context as it now stands.
pub fn expanded(added: usize, missing: &[String], context: &str) -> String {
    let mut text = format!("{added} declaration(s) added");
    if !missing.is_empty() {
        text.push_str(&format!(". Not found: {}", missing.join(", ")));
    }
    text.push_str(&format!("\n\n{EXPAND_INTRO}\n\n```lean\n{context}\n```"));
    text
}

/// The result of `focus`: the new context, and the target the session now has.
pub fn focused(
    count: usize,
    missing: &[String],
    target: Option<&str>,
    context: &str,
) -> String {
    let mut text = format!("The context is now these {count} declaration(s)");
    if !missing.is_empty() {
        text.push_str(&format!(". Not found: {}", missing.join(", ")));
    }
    text.push('.');
    text.push_str(&format!(
        "\n\n{}",
        match target {
            Some(name) => target_intro(name),
            None => "There is no target term.".to_string(),
        }
    ));
    text.push_str(&format!("\n\n{FOCUS_INTRO}\n\n```lean\n{context}\n```"));
    text
}

/// The result of `modify_ast`, once the change is on disk.
///
/// The whole declaration as it now reads is included: the model has to be able to
/// see what its change did to the file, not be told that it worked.
pub fn modified(action: &str, name: &str, file: &str, text: &str) -> String {
    if action == "remove" {
        return format!("Removed `{name}` from {file}.");
    }
    format!("{action} `{name}` in {file}. It now reads:\n\n```lean\n{text}\n```")
}

/// The result of an `ask` the user never answered.
pub const NO_ANSWER: &str = "The user did not answer.";

// ------------------------------------------------------------------- tools

/// Descriptions shown to the model for each tool.
pub mod tool {
    pub const VIEW_AST: &str = "\
Show the project's syntax tree as text, so you can find declarations to expand, or the names \
`modify_ast` takes. Returns one line per declaration: kind, fully qualified name, file and \
signature. Use `query` to filter by name, namespace or file path.";

    pub const EXPAND: &str = "\
Add declarations to this session's context by their fully qualified name (namespace plus \
name, for example `Demo.foo`). Returns the new context, which includes the added \
declarations and everything they depend on.";

    pub const FOCUS: &str = "\
Replace this session's context with only the declarations you name — plus everything they \
depend on — and drop the rest. Use it to stop carrying a context that has grown too wide. \
Passing `target` also sets the term the session focuses on (an empty string clears it).";

    pub const MODIFY_AST: &str = "\
Change the project's declarations. `action` is `add`, `edit` or `remove`. `edit` and `remove` \
take the fully qualified name of an existing declaration (as `view_ast` lists it); `add` takes \
the file to write to and the new Lean source, and `after` to place it right after an existing \
declaration (`Demo.foo`) instead of at the end of the file. The file is written immediately.";

    pub const LEAN_CHECK: &str = "\
Ask Lean for the type of an expression, with the whole project in scope — every module the \
project has built is imported first. This is how you confirm what a name means or what a term \
is before you write about it. Returns Lean's own `#check` output: one line per message, and any \
error Lean reported.";

    pub const ASK: &str = "\
Ask the user a question and offer answers. The answer the user picks comes back to you as \
the result of this call. Use it anytime a decision cannot be backed by the axioms presented in LCONTEXT.";

    pub const ASK_MULTI: &str = "\
Ask the user a question where more than one answer may apply. All the answers the user \
picks come back to you as the result of this call. Use it anytime a conclusion cannot be backed by the axioms presented in LCONTEXT.";
}

/// Descriptions of the parameters, per tool.
pub mod param {
    pub const QUERY: &str = "Filter: part of a name, a namespace, or a file path. Empty lists the most important declarations.";
    pub const DEPTH: &str = "How deep to indent the listing (default 1).";
    pub const NAMES: &str = "Fully qualified names to add, e.g. [\"Demo.foo\", \"Demo.bar\"].";
    pub const FOCUS_NODES: &str = "The only declarations the context should hold, by fully qualified name, e.g. [\"Demo.foo\"].";
    pub const TARGET: &str = "The declaration this session focuses on, by fully qualified name. An empty string clears the target.";
    pub const ACTION: &str = "What to do: `add`, `edit` or `remove`.";
    pub const NODE: &str = "Fully qualified name of the declaration to edit or remove, e.g. `Demo.foo`.";
    pub const FILE: &str = "File to write the new declaration to, e.g. `Demo/Basic.lean`.";
    pub const TEXT: &str = "The declaration's new Lean source, exactly as it should be written to the file.";
    pub const AFTER: &str = "Fully qualified name of the declaration the new one goes right after. Empty puts it at the end of the file.";
    pub const EXPRESSION: &str = "The Lean term to check, e.g. `Demo.foo` or `(fun x : Nat => x + 1)`.";
    pub const QUESTION: &str = "The question to ask the user.";
    pub const OPTIONS: &str = "The answers the user may pick from.";
}
