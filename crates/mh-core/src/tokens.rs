pub fn estimate_tokens(text: &str) -> u64 {
    tiktoken_rs::o200k_base_singleton().count_ordinary(text) as u64
}

pub fn context_window_for_model(model: Option<&str>, fallback: u64) -> u64 {
    let model = model.unwrap_or_default().to_ascii_lowercase();
    if model
        .strip_prefix("gpt-6")
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(['.', '-']))
    {
        1_000_000
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_multilingual_text_and_uses_model_context_limits() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("Hello, world!"), 4);
        assert_eq!(estimate_tokens("你好"), 1);
        assert_eq!(
            context_window_for_model(Some("gpt-6.2-sol"), 200_000),
            1_000_000
        );
        assert_eq!(context_window_for_model(Some("gpt-60"), 200_000), 200_000);
    }
}
