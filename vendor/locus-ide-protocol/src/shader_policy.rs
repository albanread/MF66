pub const MAX_SHADER_FOR_LOOP_BOUND: u32 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShaderPolicyError {
    PreprocessorDirective {
        offset: usize,
    },
    UnterminatedBlockComment {
        offset: usize,
    },
    UnterminatedStringLiteral {
        offset: usize,
    },
    StringLiteralNotAllowed {
        offset: usize,
    },
    NonAsciiSourceCharacter {
        offset: usize,
    },
    ForbiddenIdentifier {
        ident: String,
        offset: usize,
    },
    UnbalancedDelimiter {
        delimiter: char,
        offset: usize,
    },
    MismatchedDelimiter {
        open: char,
        close: char,
        offset: usize,
    },
    MissingMainImage,
    MultipleMainImageEntries,
    InvalidMainImageSignature,
    MalformedForLoop {
        offset: usize,
    },
    ForLoopMissingLiteralBound {
        offset: usize,
    },
    ForLoopBoundTooLarge {
        bound: u32,
        max: u32,
        offset: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind {
    Ident,
    Number,
    Symbol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    text: String,
    offset: usize,
}

pub fn validate_fragment_shader_policy(source: &str) -> Result<(), ShaderPolicyError> {
    let tokens = lex_policy_tokens(source)?;
    reject_forbidden_identifiers(&tokens)?;
    validate_delimiters(&tokens)?;
    validate_main_image_entry(&tokens)?;
    validate_for_loops(&tokens)
}

fn lex_policy_tokens(source: &str) -> Result<Vec<Token>, ShaderPolicyError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let start = i;
            i += 2;
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(ShaderPolicyError::UnterminatedBlockComment { offset: start });
            }
            continue;
        }
        if b == b'#' {
            return Err(ShaderPolicyError::PreprocessorDirective { offset: i });
        }
        if b == b'"' || b == b'\'' {
            return reject_string_literal(bytes, i);
        }
        if !b.is_ascii() {
            return Err(ShaderPolicyError::NonAsciiSourceCharacter { offset: i });
        }
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Ident,
                text: source[start..i].to_string(),
                offset: start,
            });
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_number_continue(bytes[i]) {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Number,
                text: source[start..i].to_string(),
                offset: start,
            });
            continue;
        }

        let start = i;
        let text = match (bytes.get(i), bytes.get(i + 1)) {
            (Some(b'<'), Some(b'='))
            | (Some(b'>'), Some(b'='))
            | (Some(b'='), Some(b'='))
            | (Some(b'!'), Some(b'='))
            | (Some(b'+'), Some(b'+'))
            | (Some(b'-'), Some(b'-'))
            | (Some(b'&'), Some(b'&'))
            | (Some(b'|'), Some(b'|')) => {
                i += 2;
                source[start..i].to_string()
            }
            _ => {
                i += 1;
                source[start..i].to_string()
            }
        };
        tokens.push(Token {
            kind: TokenKind::Symbol,
            text,
            offset: start,
        });
    }
    Ok(tokens)
}

fn reject_string_literal<T>(bytes: &[u8], start: usize) -> Result<T, ShaderPolicyError> {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return Err(ShaderPolicyError::StringLiteralNotAllowed { offset: start });
        }
        i += 1;
    }
    Err(ShaderPolicyError::UnterminatedStringLiteral { offset: start })
}

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_continue(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn is_number_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-')
}

fn reject_forbidden_identifiers(tokens: &[Token]) -> Result<(), ShaderPolicyError> {
    for token in tokens {
        if token.kind == TokenKind::Ident && is_forbidden_identifier(&token.text) {
            return Err(ShaderPolicyError::ForbiddenIdentifier {
                ident: token.text.clone(),
                offset: token.offset,
            });
        }
    }
    Ok(())
}

fn is_forbidden_identifier(ident: &str) -> bool {
    matches!(
        ident,
        "while"
            | "do"
            | "groupshared"
            | "globallycoherent"
            | "numthreads"
            | "SV_DispatchThreadID"
            | "SV_GroupID"
            | "SV_GroupThreadID"
            | "SV_GroupIndex"
            | "RWBuffer"
            | "RWByteAddressBuffer"
            | "AppendStructuredBuffer"
            | "ConsumeStructuredBuffer"
            | "StructuredBuffer"
            | "ByteAddressBuffer"
            | "Texture1D"
            | "Texture1DArray"
            | "Texture2D"
            | "Texture2DArray"
            | "Texture3D"
            | "TextureCube"
            | "TextureCubeArray"
            | "SamplerState"
            | "SamplerComparisonState"
            | "sampler"
            | "sampler1D"
            | "sampler2D"
            | "sampler3D"
            | "samplerCUBE"
    ) || ident.starts_with("RWTexture")
        || ident.starts_with("RasterizerOrdered")
        || ident.starts_with("Interlocked")
}

fn validate_delimiters(tokens: &[Token]) -> Result<(), ShaderPolicyError> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    for token in tokens {
        if token.kind != TokenKind::Symbol {
            continue;
        }
        let Some(ch) = single_symbol(&token.text) else {
            continue;
        };
        match ch {
            '(' | '[' | '{' => stack.push((ch, token.offset)),
            ')' | ']' | '}' => {
                let Some((open, _)) = stack.pop() else {
                    return Err(ShaderPolicyError::UnbalancedDelimiter {
                        delimiter: ch,
                        offset: token.offset,
                    });
                };
                if matching_close(open) != ch {
                    return Err(ShaderPolicyError::MismatchedDelimiter {
                        open,
                        close: ch,
                        offset: token.offset,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some((delimiter, offset)) = stack.pop() {
        return Err(ShaderPolicyError::UnbalancedDelimiter { delimiter, offset });
    }
    Ok(())
}

fn single_symbol(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn matching_close(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => open,
    }
}

fn validate_main_image_entry(tokens: &[Token]) -> Result<(), ShaderPolicyError> {
    let entries: Vec<_> = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.kind == TokenKind::Ident && token.text == "main_image")
        .map(|(idx, _)| idx)
        .collect();
    match entries.len() {
        0 => return Err(ShaderPolicyError::MissingMainImage),
        1 => {}
        _ => return Err(ShaderPolicyError::MultipleMainImageEntries),
    }
    let idx = entries[0];
    if !matches_ident(tokens, idx.wrapping_sub(1), "float4")
        || !matches_symbol(tokens, idx + 1, "(")
        || !matches_ident(tokens, idx + 2, "float2")
        || !tokens
            .get(idx + 3)
            .is_some_and(|token| token.kind == TokenKind::Ident)
        || !matches_symbol(tokens, idx + 4, ")")
    {
        return Err(ShaderPolicyError::InvalidMainImageSignature);
    }
    Ok(())
}

fn matches_ident(tokens: &[Token], idx: usize, text: &str) -> bool {
    tokens
        .get(idx)
        .is_some_and(|token| token.kind == TokenKind::Ident && token.text == text)
}

fn matches_symbol(tokens: &[Token], idx: usize, text: &str) -> bool {
    tokens
        .get(idx)
        .is_some_and(|token| token.kind == TokenKind::Symbol && token.text == text)
}

fn validate_for_loops(tokens: &[Token]) -> Result<(), ShaderPolicyError> {
    let mut i = 0;
    while i < tokens.len() {
        if !matches_ident(tokens, i, "for") {
            i += 1;
            continue;
        }
        let offset = tokens[i].offset;
        let Some((open, close)) = paren_range_after(tokens, i) else {
            return Err(ShaderPolicyError::MalformedForLoop { offset });
        };
        let header = &tokens[open + 1..close];
        validate_for_header(header, offset)?;
        i = close + 1;
    }
    Ok(())
}

fn paren_range_after(tokens: &[Token], idx: usize) -> Option<(usize, usize)> {
    if !matches_symbol(tokens, idx + 1, "(") {
        return None;
    }
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(idx + 1) {
        if token.kind != TokenKind::Symbol {
            continue;
        }
        match token.text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((idx + 1, i));
                }
            }
            _ => {}
        }
    }
    None
}

fn validate_for_header(header: &[Token], offset: usize) -> Result<(), ShaderPolicyError> {
    let semicolons: Vec<_> = header
        .iter()
        .enumerate()
        .filter(|(_, token)| token.kind == TokenKind::Symbol && token.text == ";")
        .map(|(idx, _)| idx)
        .collect();
    if semicolons.len() != 2 {
        return Err(ShaderPolicyError::MalformedForLoop { offset });
    }
    let condition = &header[semicolons[0] + 1..semicolons[1]];
    let Some((bound, bound_offset)) = literal_loop_bound(condition) else {
        return Err(ShaderPolicyError::ForLoopMissingLiteralBound { offset });
    };
    if bound > MAX_SHADER_FOR_LOOP_BOUND {
        return Err(ShaderPolicyError::ForLoopBoundTooLarge {
            bound,
            max: MAX_SHADER_FOR_LOOP_BOUND,
            offset: bound_offset,
        });
    }
    Ok(())
}

fn literal_loop_bound(condition: &[Token]) -> Option<(u32, usize)> {
    for window in condition.windows(3) {
        if window[1].kind != TokenKind::Symbol {
            continue;
        }
        let op = window[1].text.as_str();
        if !matches!(op, "<" | "<=" | ">" | ">=") {
            continue;
        }
        if window[0].kind == TokenKind::Ident && window[2].kind == TokenKind::Number {
            return parse_u32_literal(&window[2].text).map(|bound| (bound, window[2].offset));
        }
        if window[0].kind == TokenKind::Number && window[2].kind == TokenKind::Ident {
            return parse_u32_literal(&window[0].text).map(|bound| (bound, window[0].offset));
        }
    }
    None
}

fn parse_u32_literal(text: &str) -> Option<u32> {
    let trimmed = text.trim_end_matches(|ch: char| matches!(ch, 'u' | 'U' | 'l' | 'L'));
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        u32::from_str_radix(&trimmed[2..], 16).ok()
    } else if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        trimmed.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_SHADER: &str = r#"
float4 main_image(float2 fragCoord) {
  float3 color = float3(0.2, 0.4, 0.8);
  for (int n = 0; n < 64; n = n + 1) {
    color.xy = color.yx;
  }
  return float4(color, 1.0);
}
"#;

    #[test]
    fn accepts_restricted_main_image_shader() {
        assert_eq!(validate_fragment_shader_policy(SIMPLE_SHADER), Ok(()));
    }

    #[test]
    fn ignores_forbidden_words_inside_comments() {
        let src = r#"
// RWTexture2D and while are harmless in comments.
/* #include also harmless in comments. */
float4 main_image(float2 p) { return float4(1, 0, 0, 1); }
"#;
        assert_eq!(validate_fragment_shader_policy(src), Ok(()));
    }

    #[test]
    fn rejects_preprocessor_and_resource_declarations() {
        assert!(matches!(
            validate_fragment_shader_policy(
                "#include \"x\"\nfloat4 main_image(float2 p){return 1;}"
            ),
            Err(ShaderPolicyError::PreprocessorDirective { .. })
        ));
        assert!(matches!(
            validate_fragment_shader_policy(
                "Texture2D data; float4 main_image(float2 p){return 1;}"
            ),
            Err(ShaderPolicyError::ForbiddenIdentifier { ident, .. }) if ident == "Texture2D"
        ));
        assert!(matches!(
            validate_fragment_shader_policy(
                "RWTexture2D<float4> outp; float4 main_image(float2 p){return 1;}"
            ),
            Err(ShaderPolicyError::ForbiddenIdentifier { ident, .. }) if ident == "RWTexture2D"
        ));
    }

    #[test]
    fn rejects_unbounded_loop_forms() {
        assert!(matches!(
            validate_fragment_shader_policy("float4 main_image(float2 p){while(true){ } return 1;}"),
            Err(ShaderPolicyError::ForbiddenIdentifier { ident, .. }) if ident == "while"
        ));
        assert!(matches!(
            validate_fragment_shader_policy(
                "float4 main_image(float2 p){for(int n=0;n<limit;n=n+1){} return 1;}"
            ),
            Err(ShaderPolicyError::ForLoopMissingLiteralBound { .. })
        ));
        assert!(matches!(
            validate_fragment_shader_policy(
                "float4 main_image(float2 p){for(int n=0;n<4097;n=n+1){} return 1;}"
            ),
            Err(ShaderPolicyError::ForLoopBoundTooLarge { bound: 4097, .. })
        ));
    }

    #[test]
    fn rejects_missing_or_wrong_entry() {
        assert_eq!(
            validate_fragment_shader_policy("float4 helper(float2 p){return 1;}"),
            Err(ShaderPolicyError::MissingMainImage)
        );
        assert_eq!(
            validate_fragment_shader_policy("float3 main_image(float2 p){return 1;}"),
            Err(ShaderPolicyError::InvalidMainImageSignature)
        );
    }

    #[test]
    fn rejects_string_literals_and_unbalanced_delimiters() {
        assert!(matches!(
            validate_fragment_shader_policy("float4 main_image(float2 p){ return \"x\"; }"),
            Err(ShaderPolicyError::StringLiteralNotAllowed { .. })
        ));
        assert!(matches!(
            validate_fragment_shader_policy("float4 main_image(float2 p){ return 1; "),
            Err(ShaderPolicyError::UnbalancedDelimiter { delimiter: '{', .. })
        ));
    }
}
