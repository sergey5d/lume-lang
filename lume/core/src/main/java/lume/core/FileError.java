package lume.core;

public sealed interface FileError permits
        FileError.NotFound,
        FileError.AccessDenied,
        FileError.Closed,
        FileError.InvalidEncoding,
        FileError.IoFailure {

    record NotFound(String path) implements FileError {}

    record AccessDenied(String path) implements FileError {}

    record Closed(String path) implements FileError {}

    record InvalidEncoding(String path, long offset) implements FileError {}

    record IoFailure(String operation, String path, String message) implements FileError {}
}
