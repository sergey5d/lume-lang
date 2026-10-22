package lume.core;

public record RegexError(String message, Option<Long> offset) {}
