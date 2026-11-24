//! A small POSIX shell lexer, good enough to find every simple command in a command
//! line: pipelines, lists, subshells, `$(…)` and backticks, redirections and heredocs.
//! It never executes anything; when in doubt it reports an error and the caller treats
//! the command as risky.

/// One simple command: its words (quotes removed) and output redirections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimpleCommand {
    pub words: Vec<String>,
    /// Targets of `>`, `>>`, `&>`, `>|` and `N>` redirections.
    pub writes: Vec<String>,
    /// Standard input comes from the previous command of a pipeline.
    pub piped: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub commands: Vec<SimpleCommand>,
    /// Bodies of `$(…)`, `` `…` `` and `<(…)`, to be analysed on their own.
    pub substitutions: Vec<String>,
    pub error: Option<String>,
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    parsed: Parsed,
    current: SimpleCommand,
    word: String,
    /// The word has content, even if empty (`''`).
    in_word: bool,
    /// The next word is a redirection target.
    redirect: Option<bool>,
    /// Heredoc delimiters waiting for the end of the line, with their `<<-` flag.
    heredocs: Vec<(String, bool)>,
    /// The next word is a heredoc delimiter, with its `<<-` flag.
    heredoc_next: Option<bool>,
    _input: &'a str,
}

pub fn parse(input: &str) -> Parsed {
    let mut lexer = Lexer {
        chars: input.chars().collect(),
        pos: 0,
        parsed: Parsed::default(),
        current: SimpleCommand::default(),
        word: String::new(),
        in_word: false,
        redirect: None,
        heredocs: Vec::new(),
        heredoc_next: None,
        _input: input,
    };
    lexer.run();
    lexer.parsed
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn fail(&mut self, message: &str) {
        if self.parsed.error.is_none() {
            self.parsed.error = Some(message.to_string());
        }
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if let Some(strip) = self.heredoc_next.take() {
            self.heredocs.push((word, strip));
        } else if let Some(write) = self.redirect.take() {
            if write {
                self.current.writes.push(word);
            }
        } else {
            self.current.words.push(word);
        }
    }

    /// Ends the current simple command; `piped` marks the next one as reading a pipe.
    fn end_command(&mut self, piped: bool) {
        self.end_word();
        if self.redirect.take().is_some() {
            self.fail("redirection without a target");
        }
        let command = std::mem::take(&mut self.current);
        if !command.words.is_empty() || !command.writes.is_empty() {
            self.parsed.commands.push(command);
        }
        self.current.piped = piped;
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' => {
                    self.end_word();
                    self.pos += 1;
                }
                '\n' => {
                    self.end_command(false);
                    self.pos += 1;
                    self.skip_heredocs();
                }
                '#' if !self.in_word => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                ';' => {
                    self.end_command(false);
                    self.pos += 1;
                    if self.peek(0) == Some(';') {
                        self.pos += 1;
                    }
                }
                '&' => {
                    if self.peek(1) == Some('>') {
                        self.end_word();
                        self.pos += 2;
                        if self.peek(0) == Some('>') {
                            self.pos += 1;
                        }
                        self.redirect = Some(true);
                    } else {
                        self.end_command(false);
                        self.pos += if self.peek(1) == Some('&') { 2 } else { 1 };
                    }
                }
                '|' => {
                    let double = self.peek(1) == Some('|');
                    self.end_command(!double);
                    self.pos += if double || self.peek(1) == Some('&') {
                        2
                    } else {
                        1
                    };
                }
                '(' | ')' if !self.in_word => {
                    self.end_command(false);
                    self.pos += 1;
                }
                '>' | '<' => self.redirection(c),
                '\'' => {
                    self.in_word = true;
                    self.pos += 1;
                    loop {
                        match self.peek(0) {
                            Some('\'') => break,
                            Some(c) => self.word.push(c),
                            None => return self.fail("unterminated single quote"),
                        }
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                '"' => {
                    self.in_word = true;
                    self.pos += 1;
                    loop {
                        match self.peek(0) {
                            Some('"') => break,
                            Some('\\') => {
                                if let Some(next) = self.peek(1) {
                                    if !matches!(next, '"' | '\\' | '$' | '`' | '\n') {
                                        self.word.push('\\');
                                    }
                                    if next != '\n' {
                                        self.word.push(next);
                                    }
                                    self.pos += 1;
                                }
                            }
                            Some('$') if self.peek(1) == Some('(') => {
                                self.substitution();
                                continue;
                            }
                            Some('`') => {
                                self.backtick();
                                continue;
                            }
                            Some(c) => self.word.push(c),
                            None => return self.fail("unterminated double quote"),
                        }
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                '\\' => {
                    self.pos += 1;
                    match self.peek(0) {
                        Some('\n') => {}
                        Some(c) => {
                            self.in_word = true;
                            self.word.push(c);
                        }
                        None => {}
                    }
                    self.pos += 1;
                }
                '$' if self.peek(1) == Some('(') => self.substitution(),
                '`' => self.backtick(),
                c => {
                    self.in_word = true;
                    self.word.push(c);
                    self.pos += 1;
                }
            }
            if self.parsed.error.is_some() {
                return;
            }
        }
        self.end_command(false);
        if !self.heredocs.is_empty() {
            self.fail("unterminated heredoc");
        }
    }

    fn redirection(&mut self, c: char) {
        let fd_prefix = self.in_word && self.word.chars().all(|d| d.is_ascii_digit());
        if fd_prefix {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
        if c == '<' && self.peek(1) == Some('(') {
            self.pos += 1;
            self.substitution_from('(');
            return;
        }
        if c == '>' && self.peek(1) == Some('(') {
            self.pos += 1;
            self.substitution_from('(');
            return;
        }
        self.pos += 1;
        if c == '<' {
            if self.peek(0) == Some('<') {
                self.pos += 1;
                if self.peek(0) == Some('<') {
                    self.pos += 1;
                    self.redirect = Some(false);
                    return;
                }
                let strip = self.peek(0) == Some('-');
                if strip {
                    self.pos += 1;
                }
                self.heredoc_next = Some(strip);
                return;
            }
            if self.peek(0) == Some('&') || self.peek(0) == Some('>') {
                self.pos += 1;
            }
            self.redirect = Some(false);
            return;
        }
        if matches!(self.peek(0), Some('>') | Some('|')) {
            self.pos += 1;
        }
        if self.peek(0) == Some('&') {
            self.pos += 1;
            while self.peek(0).is_some_and(|d| d.is_ascii_digit() || d == '-') {
                self.pos += 1;
            }
            return;
        }
        self.redirect = Some(true);
    }

    /// Skips heredoc bodies after a newline.
    fn skip_heredocs(&mut self) {
        for (delimiter, strip) in std::mem::take(&mut self.heredocs) {
            loop {
                if self.pos >= self.chars.len() {
                    self.fail("unterminated heredoc");
                    return;
                }
                let start = self.pos;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
                let line: String = self.chars[start..self.pos].iter().collect();
                self.pos += 1;
                let line = if strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == delimiter {
                    break;
                }
            }
        }
    }

    fn substitution(&mut self) {
        self.pos += 1;
        self.substitution_from('(');
    }

    /// Captures the text up to the parenthesis matching the one at `pos`.
    fn substitution_from(&mut self, _open: char) {
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1;
        let mut quote: Option<char> = None;
        while let Some(c) = self.peek(0) {
            match (quote, c) {
                (Some('\''), '\'') | (Some('"'), '"') => quote = None,
                (Some('"'), '\\') => self.pos += 1,
                (Some(_), _) => {}
                (None, '\'' | '"') => quote = Some(c),
                (None, '\\') => self.pos += 1,
                (None, '(') => depth += 1,
                (None, ')') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        if depth != 0 {
            return self.fail("unterminated $(");
        }
        let body: String = self.chars[start..self.pos].iter().collect();
        self.pos += 1;
        self.parsed.substitutions.push(body);
        self.in_word = true;
        self.word.push_str("$(…)");
    }

    fn backtick(&mut self) {
        self.pos += 1;
        let start = self.pos;
        while let Some(c) = self.peek(0) {
            if c == '`' {
                break;
            }
            if c == '\\' {
                self.pos += 1;
            }
            self.pos += 1;
        }
        if self.peek(0) != Some('`') {
            return self.fail("unterminated backtick");
        }
        let body: String = self.chars[start..self.pos].iter().collect();
        self.pos += 1;
        self.parsed.substitutions.push(body);
        self.in_word = true;
        self.word.push_str("$(…)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(input: &str) -> Vec<Vec<String>> {
        let parsed = parse(input);
        assert_eq!(parsed.error, None, "{input}");
        parsed.commands.into_iter().map(|c| c.words).collect()
    }

    #[test]
    fn splits_lists_and_pipelines() {
        assert_eq!(
            words("cd /app && git pull; ls -la | grep x || echo 'a b'"),
            vec![
                vec!["cd", "/app"],
                vec!["git", "pull"],
                vec!["ls", "-la"],
                vec!["grep", "x"],
                vec!["echo", "a b"],
            ]
        );
        let parsed = parse("curl -s x | sudo bash");
        assert!(!parsed.commands[0].piped);
        assert!(parsed.commands[1].piped);
    }

    #[test]
    fn quotes_escapes_and_comments() {
        assert_eq!(
            words(r#"echo "a \"b\" $HOME" 'c;d' e\ f # rm -rf /"#),
            vec![vec!["echo", r#"a "b" $HOME"#, "c;d", "e f"]]
        );
        assert_eq!(words("echo ''"), vec![vec!["echo", ""]]);
    }

    #[test]
    fn redirections_and_heredocs() {
        let parsed =
            parse("echo hi > /etc/motd 2>/dev/null; cat <<'EOF' >> out.txt\nrm -rf /\nEOF\nls");
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.commands.len(), 3);
        assert_eq!(parsed.commands[0].writes, vec!["/etc/motd", "/dev/null"]);
        assert_eq!(parsed.commands[1].words, vec!["cat"]);
        assert_eq!(parsed.commands[1].writes, vec!["out.txt"]);
        assert_eq!(parsed.commands[2].words, vec!["ls"]);
        let parsed = parse("cmd 2>&1 &>log");
        assert_eq!(parsed.commands[0].writes, vec!["log"]);
    }

    #[test]
    fn substitutions_are_collected() {
        let parsed = parse("echo $(rm -rf /tmp/x) `id` \"$(date)\"; (cd a; make)");
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.substitutions, vec!["rm -rf /tmp/x", "id", "date"]);
        assert_eq!(parsed.commands.len(), 3);
    }

    #[test]
    fn broken_input_is_an_error() {
        assert!(parse("echo 'oops").error.is_some());
        assert!(parse("echo $(oops").error.is_some());
        assert!(parse("cat <<EOF\nno end").error.is_some());
    }
}
