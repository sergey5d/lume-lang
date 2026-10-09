package lume.core;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

public final class LumeTextFileReader implements LumeTextReader {
    private final String path;
    private InputStream input;
    private long position;

    LumeTextFileReader(String path) throws IOException {
        this.path = path;
        this.input = Files.newInputStream(Path.of(path));
    }

    public String path() {
        return path;
    }

    @Override
    public Boolean closed() {
        return input == null;
    }

    @Override
    public Result<Option<String>, FileError> readLine() {
        if (input == null) {
            return LumeFile.err(new FileError.Closed(path));
        }
        long start = position;
        var output = new ByteArrayOutputStream();
        try {
            while (true) {
                int value = input.read();
                if (value < 0) {
                    if (output.size() == 0) {
                        return LumeFile.ok(LumeRuntime.optionNone());
                    }
                    return decodeLine(output.toByteArray(), start);
                }
                position++;
                if (value == '\n') {
                    var bytes = output.toByteArray();
                    if (bytes.length > 0 && bytes[bytes.length - 1] == '\r') {
                        bytes = Arrays.copyOf(bytes, bytes.length - 1);
                    }
                    return decodeLine(bytes, start);
                }
                output.write(value);
            }
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("readLine", path, error));
        }
    }

    @Override
    public Result<String, FileError> readToEnd() {
        if (input == null) {
            return LumeFile.err(new FileError.Closed(path));
        }
        long start = position;
        try {
            var bytes = input.readAllBytes();
            position = Math.addExact(position, bytes.length);
            return LumeFile.decodeText(path, bytes, start);
        } catch (ArithmeticException error) {
            return LumeFile.err(new FileError.IoFailure(
                    "readToEnd", path, "file position exceeds Int range"));
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("readToEnd", path, error));
        }
    }

    @Override
    public Result<LumeUnit, FileError> close() {
        if (input == null) {
            return LumeFile.ok(LumeUnit.INSTANCE);
        }
        try {
            input.close();
            input = null;
            return LumeFile.ok(LumeUnit.INSTANCE);
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("close", path, error));
        }
    }

    private Result<Option<String>, FileError> decodeLine(byte[] bytes, long start) {
        var decoded = LumeFile.decodeText(path, bytes, start);
        if (decoded instanceof Result.Ok<?, ?> ok) {
            return LumeFile.ok(LumeRuntime.optionSome((String) ok.value()));
        }
        var error = (Result.Err<?, ?>) decoded;
        return LumeFile.err((FileError) error.error());
    }
}
