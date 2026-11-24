//! Lexer and parser of the shader expression language.
//!
//! A program is a list of `let name = expr;` bindings (a later one may rebind a name) followed by
//! one final expression, the color. Expressions have numbers, `#rgb`/`#rrggbb`/`#rrggbbaa` colors, names, function calls,
//! parentheses, unary minus, `+ - * / %`, and component swizzles (`.x`, `.rgb`, `.yx`, ...).
//! `//` starts a comment to the end of the line. Every error names its line and column.

use anyhow::{bail, Result};

/// Largest program accepted, in bytes.
pub const MAX_SOURCE: usize = 16 * 1024;
/// Deepest nesting of parentheses and calls.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for Pos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f32),
    Color(Vec<f32>),
    Name(String),
    Let,
    Symbol(char),
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Number(f32),
    Color(Vec<f32>),
    Name(String, Pos),
    Call(String, Vec<Expr>, Pos),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>, Pos),
    Swizzle(Box<Expr>, Vec<usize>, Pos),
}

#[derive(Debug)]
pub struct Program {
    pub bindings: Vec<(String, Expr, Pos)>,
    pub result: Expr,
}

fn lex(source: &str) -> Result<Vec<(Token, Pos)>> {
    if source.len() > MAX_SOURCE {
        bail!(
            "the program is {} bytes; at most {MAX_SOURCE} are allowed",
            source.len()
        );
    }
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let (mut i, mut line, mut column) = (0, 1_u32, 1_u32);
    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, column };
        let start = i;
        if c == '\n' {
            line += 1;
            column = 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            column += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        let token = if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value: f32 = text
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number {text:?} at {pos}"))?;
            if !value.is_finite() {
                bail!("number {text:?} at {pos} is out of range");
            }
            Token::Number(value)
        } else if c == '#' {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let hex: String = chars[start + 1..i].iter().collect();
            Token::Color(hex_color(&hex).ok_or_else(|| {
                anyhow::anyhow!("invalid color #{hex} at {pos}; use #rgb, #rrggbb, or #rrggbbaa")
            })?)
        } else if c.is_ascii_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            if name == "let" {
                Token::Let
            } else {
                Token::Name(name)
            }
        } else if "+-*/%(),;=.".contains(c) {
            i += 1;
            Token::Symbol(c)
        } else {
            bail!("unexpected character {c:?} at {pos}");
        };
        column += (i - start) as u32;
        tokens.push((token, pos));
    }
    tokens.push((Token::End, Pos { line, column }));
    Ok(tokens)
}

/// `#rgb`, `#rrggbb`, or `#rrggbbaa` as sRGB components in `[0, 1]`.
fn hex_color(hex: &str) -> Option<Vec<f32>> {
    let digits: Vec<u32> = hex.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
    let bytes: Vec<u32> = match digits.len() {
        3 => digits.iter().map(|d| d * 17).collect(),
        6 | 8 => digits.chunks(2).map(|p| p[0] * 16 + p[1]).collect(),
        _ => return None,
    };
    Some(bytes.into_iter().map(|b| b as f32 / 255.0).collect())
}

struct Parser {
    tokens: Vec<(Token, Pos)>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at].0
    }

    fn pos(&self) -> Pos {
        self.tokens[self.at].1
    }

    fn next(&mut self) -> (Token, Pos) {
        let token = self.tokens[self.at].clone();
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
        token
    }

    fn expect(&mut self, symbol: char) -> Result<()> {
        let (token, pos) = self.next();
        if token != Token::Symbol(symbol) {
            bail!("expected '{symbol}' at {pos}, found {}", describe(&token));
        }
        Ok(())
    }

    fn program(&mut self) -> Result<Program> {
        let mut bindings = Vec::new();
        while *self.peek() == Token::Let {
            self.next();
            let (token, pos) = self.next();
            let Token::Name(name) = token else {
                bail!("expected a name after 'let' at {pos}");
            };
            self.expect('=')?;
            let value = self.expr()?;
            self.expect(';')?;
            bindings.push((name, value, pos));
        }
        if *self.peek() == Token::End {
            bail!("the program has no final color expression");
        }
        let result = self.expr()?;
        if *self.peek() == Token::Symbol(';') {
            self.next();
        }
        let (token, pos) = self.next();
        if token != Token::End {
            bail!(
                "unexpected {} at {pos}; only the last expression gives the color (bind others with let)",
                describe(&token)
            );
        }
        Ok(Program { bindings, result })
    }

    fn expr(&mut self) -> Result<Expr> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            bail!(
                "expression nested deeper than {MAX_DEPTH} levels at {}",
                self.pos()
            );
        }
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Token::Symbol('+') => BinOp::Add,
                Token::Symbol('-') => BinOp::Sub,
                _ => break,
            };
            let pos = self.next().1;
            let right = self.term()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right), pos);
        }
        self.depth -= 1;
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Token::Symbol('*') => BinOp::Mul,
                Token::Symbol('/') => BinOp::Div,
                Token::Symbol('%') => BinOp::Mod,
                _ => break,
            };
            let pos = self.next().1;
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right), pos);
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr> {
        if *self.peek() == Token::Symbol('-') {
            self.next();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if *self.peek() == Token::Symbol('+') {
            self.next();
            return self.unary();
        }
        let mut value = self.primary()?;
        while *self.peek() == Token::Symbol('.') {
            let pos = self.next().1;
            let (token, _) = self.next();
            let Token::Name(components) = token else {
                bail!("expected components such as .x or .rgb after '.' at {pos}");
            };
            value = Expr::Swizzle(Box::new(value), swizzle(&components, pos)?, pos);
        }
        Ok(value)
    }

    fn primary(&mut self) -> Result<Expr> {
        let (token, pos) = self.next();
        match token {
            Token::Number(value) => Ok(Expr::Number(value)),
            Token::Color(components) => Ok(Expr::Color(components)),
            Token::Symbol('(') => {
                let inner = self.expr()?;
                self.expect(')')?;
                Ok(inner)
            }
            Token::Name(name) => {
                if *self.peek() != Token::Symbol('(') {
                    return Ok(Expr::Name(name, pos));
                }
                self.next();
                let mut args = Vec::new();
                if *self.peek() != Token::Symbol(')') {
                    loop {
                        args.push(self.expr()?);
                        if *self.peek() == Token::Symbol(',') {
                            self.next();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(')')?;
                Ok(Expr::Call(name, args, pos))
            }
            other => bail!("expected a value at {pos}, found {}", describe(&other)),
        }
    }
}

fn swizzle(components: &str, pos: Pos) -> Result<Vec<usize>> {
    if components.is_empty() || components.len() > 4 {
        bail!("swizzle .{components} at {pos} must have 1 to 4 components");
    }
    let sets = ["xyzw", "rgba"];
    for set in sets {
        if let Some(indices) = components
            .chars()
            .map(|c| set.find(c))
            .collect::<Option<Vec<_>>>()
        {
            return Ok(indices);
        }
    }
    bail!("invalid swizzle .{components} at {pos}; use x, y, z, w or r, g, b, a (not mixed)")
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(n) => format!("number {n}"),
        Token::Color(_) => "a color".into(),
        Token::Name(name) => format!("'{name}'"),
        Token::Let => "'let'".into(),
        Token::Symbol(c) => format!("'{c}'"),
        Token::End => "the end of the program".into(),
    }
}

pub fn parse(source: &str) -> Result<Program> {
    let tokens = lex(source)?;
    Parser {
        tokens,
        at: 0,
        depth: 0,
    }
    .program()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bindings_and_a_result() {
        let program =
            parse("// sky\nlet a = uv.x * 2 + -1;\nlet c = mix(#000, #ff8800, a);\nc.rgb").unwrap();
        assert_eq!(program.bindings.len(), 2);
        assert!(matches!(program.result, Expr::Swizzle(_, ref c, _) if c == &[0, 1, 2]));
    }

    #[test]
    fn reports_positions() {
        let error = parse("let a = 1;\nlet b = (a + ;\nb")
            .unwrap_err()
            .to_string();
        assert!(error.contains("line 2"), "{error}");
        assert!(parse("1 2").is_err());
        assert!(parse("#12345").is_err());
        assert!(parse("uv.xq").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn parses_colors_and_scientific_numbers() {
        assert_eq!(hex_color("f80").unwrap(), vec![1.0, 136.0 / 255.0, 0.0]);
        assert_eq!(hex_color("ff000080").unwrap().len(), 4);
        assert!(matches!(parse("1.5e-2").unwrap().result, Expr::Number(n) if n == 0.015));
    }
}
