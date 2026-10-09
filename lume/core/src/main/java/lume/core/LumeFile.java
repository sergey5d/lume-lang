package lume.core;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.AccessDeniedException;
import java.nio.file.Files;
import java.nio.file.InvalidPathException;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;

public final class LumeFile {
    private LumeFile() {}

    public static Result<LumeVector<Long>, FileError> readBytes(String path) {
        try {
            return ok(bytes(Files.readAllBytes(Path.of(path))));
        } catch (IOException | InvalidPathException | SecurityException error) {
            return err(error("read", path, error));
        }
    }

    public static Result<String, FileError> readText(String path) {
        try {
            return decodeText(path, Files.readAllBytes(Path.of(path)), 0);
        } catch (IOException | InvalidPathException | SecurityException error) {
            return err(error("read", path, error));
        }
    }

    public static Result<LumeFileStream, FileError> open(String path) {
        try {
            return ok(new LumeFileStream(path));
        } catch (IOException | InvalidPathException | SecurityException error) {
            return err(error("open", path, error));
        }
    }

    public static Result<LumeTextFileReader, FileError> openText(String path) {
        try {
            return ok(new LumeTextFileReader(path));
        } catch (IOException | InvalidPathException | SecurityException error) {
            return err(error("open", path, error));
        }
    }

    static LumeVector<Long> bytes(byte[] values) {
        var result = LumeVector.<Long>empty();
        for (byte value : values) {
            result.add((long) Byte.toUnsignedInt(value));
        }
        return result;
    }

    static Result<String, FileError> decodeText(String path, byte[] bytes, long baseOffset) {
        var decoder = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT);
        var input = ByteBuffer.wrap(bytes);
        try {
            return ok(decoder.decode(input).toString());
        } catch (CharacterCodingException error) {
            long localOffset = input.position();
            long offset = baseOffset > Long.MAX_VALUE - localOffset
                    ? Long.MAX_VALUE
                    : baseOffset + localOffset;
            return err(new FileError.InvalidEncoding(path, offset));
        }
    }

    static FileError error(String operation, String path, Throwable error) {
        if (error instanceof NoSuchFileException) {
            return new FileError.NotFound(path);
        }
        if (error instanceof AccessDeniedException || error instanceof SecurityException) {
            return new FileError.AccessDenied(path);
        }
        var message = error.getMessage();
        return new FileError.IoFailure(
                operation,
                path,
                message == null ? error.getClass().getSimpleName() : message);
    }

    static <T> Result<T, FileError> ok(T value) {
        return new Result.Ok<>(value);
    }

    static <T> Result<T, FileError> err(FileError error) {
        return new Result.Err<>(error);
    }
}
