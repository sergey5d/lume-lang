package lume.core;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

public final class LumeFileStream implements LumeByteReader, LumeSeekable, LumeCloseable {
    private final String path;
    private FileChannel file;

    LumeFileStream(String path) throws IOException {
        this.path = path;
        this.file = FileChannel.open(Path.of(path), StandardOpenOption.READ);
    }

    public String path() {
        return path;
    }

    @Override
    public Boolean closed() {
        return file == null;
    }

    @Override
    public long position() {
        if (file == null) {
            throw new LumePanic("cannot read the position of closed file '" + path + "'");
        }
        try {
            return file.position();
        } catch (IOException error) {
            throw new LumePanic(LumeFile.error("position", path, error).toString());
        }
    }

    @Override
    public Result<LumeVector<Long>, FileError> read(long maxBytes) {
        if (file == null) {
            return LumeFile.err(new FileError.Closed(path));
        }
        if (maxBytes < 0 || maxBytes > Integer.MAX_VALUE) {
            return LumeFile.err(new FileError.IoFailure(
                    "read", path, maxBytes < 0
                            ? "maxBytes must be non-negative"
                            : "maxBytes is too large"));
        }
        try {
            var bytes = new byte[(int) maxBytes];
            int count = file.read(ByteBuffer.wrap(bytes));
            if (count < 0) {
                return LumeFile.ok(LumeVector.empty());
            }
            if (count != bytes.length) {
                bytes = java.util.Arrays.copyOf(bytes, count);
            }
            return LumeFile.ok(LumeFile.bytes(bytes));
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("read", path, error));
        }
    }

    @Override
    public Result<LumeVector<Long>, FileError> readToEnd() {
        if (file == null) {
            return LumeFile.err(new FileError.Closed(path));
        }
        try {
            var output = new ByteArrayOutputStream();
            var buffer = new byte[8192];
            int count;
            while ((count = file.read(ByteBuffer.wrap(buffer))) >= 0) {
                output.write(buffer, 0, count);
            }
            return LumeFile.ok(LumeFile.bytes(output.toByteArray()));
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("read", path, error));
        }
    }

    @Override
    public Result<Long, FileError> seek(long offset) {
        return seek(offset, SeekFrom.Start.INSTANCE);
    }

    @Override
    public Result<Long, FileError> seek(long offset, SeekFrom from) {
        if (file == null) {
            return LumeFile.err(new FileError.Closed(path));
        }
        try {
            long target;
            if (from instanceof SeekFrom.Start) {
                if (offset < 0) {
                    return LumeFile.err(new FileError.IoFailure(
                            "seek", path, "a start-relative offset cannot be negative"));
                }
                target = offset;
            } else if (from instanceof SeekFrom.Current) {
                target = Math.addExact(file.position(), offset);
            } else if (from instanceof SeekFrom.End) {
                target = Math.addExact(file.size(), offset);
            } else {
                return LumeFile.err(new FileError.IoFailure(
                        "seek", path, "unknown seek origin"));
            }
            if (target < 0) {
                return LumeFile.err(new FileError.IoFailure(
                        "seek", path, "seek position cannot be negative"));
            }
            file.position(target);
            return LumeFile.ok(target);
        } catch (ArithmeticException | IOException error) {
            return LumeFile.err(LumeFile.error("seek", path, error));
        }
    }

    @Override
    public Result<LumeUnit, FileError> close() {
        if (file == null) {
            return LumeFile.ok(LumeUnit.INSTANCE);
        }
        try {
            file.close();
            file = null;
            return LumeFile.ok(LumeUnit.INSTANCE);
        } catch (IOException error) {
            return LumeFile.err(LumeFile.error("close", path, error));
        }
    }
}
