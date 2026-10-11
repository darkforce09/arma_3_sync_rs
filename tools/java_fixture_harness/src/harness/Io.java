package harness;

import java.io.ByteArrayOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.ObjectOutputStream;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.FileTime;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.stream.Stream;
import java.util.zip.GZIPInputStream;
import java.util.zip.GZIPOutputStream;

/** File helpers shared by the fixture generators. Every listing is sorted. */
public final class Io {

	private Io() {
	}

	public static void writeString(Path file, String text) throws IOException {
		Files.createDirectories(file.toAbsolutePath().getParent());
		Files.write(file, text.getBytes(StandardCharsets.UTF_8));
	}

	public static void writeBytes(Path file, byte[] bytes, long mtimeMillis) throws IOException {
		Files.createDirectories(file.toAbsolutePath().getParent());
		Files.write(file, bytes);
		Files.setLastModifiedTime(file, FileTime.fromMillis(mtimeMillis));
	}

	/** Regular files below {@code root}, as '/'-joined relative paths sorted with {@code compareTo}. */
	public static List<String> files(Path root) throws IOException {
		List<String> files = new ArrayList<String>();
		try (Stream<Path> paths = Files.walk(root)) {
			paths.filter(p -> Files.isRegularFile(p, LinkOption.NOFOLLOW_LINKS))
					.forEach(p -> files.add(relative(root, p)));
		}
		Collections.sort(files);
		return files;
	}

	/** Directories below {@code root} that have no entry at all, sorted. */
	public static List<String> emptyDirectories(Path root) throws IOException {
		List<String> dirs = new ArrayList<String>();
		try (Stream<Path> paths = Files.walk(root)) {
			paths.filter(p -> !p.equals(root) && Files.isDirectory(p, LinkOption.NOFOLLOW_LINKS)).forEach(p -> {
				try (Stream<Path> children = Files.list(p)) {
					if (children.findAny().isEmpty()) {
						dirs.add(relative(root, p));
					}
				} catch (IOException e) {
					throw new UncheckedIOException(e);
				}
			});
		}
		Collections.sort(dirs);
		return dirs;
	}

	public static String relative(Path root, Path path) {
		return root.relativize(path).toString().replace('\\', '/');
	}

	/** Copies a tree, keeping modification times and empty directories. */
	public static void copyTree(Path source, Path target) throws IOException {
		Files.walkFileTree(source, new SimpleFileVisitor<Path>() {
			@Override
			public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) throws IOException {
				Files.createDirectories(target.resolve(source.relativize(dir).toString()));
				return FileVisitResult.CONTINUE;
			}

			@Override
			public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
				Files.copy(file, target.resolve(source.relativize(file).toString()), StandardCopyOption.COPY_ATTRIBUTES,
						StandardCopyOption.REPLACE_EXISTING);
				return FileVisitResult.CONTINUE;
			}
		});
	}

	public static void deleteTree(Path root) throws IOException {
		if (!Files.exists(root, LinkOption.NOFOLLOW_LINKS)) {
			return;
		}
		Files.walkFileTree(root, new SimpleFileVisitor<Path>() {
			@Override
			public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
				Files.delete(file);
				return FileVisitResult.CONTINUE;
			}

			@Override
			public FileVisitResult postVisitDirectory(Path dir, IOException exc) throws IOException {
				if (exc != null) {
					throw exc;
				}
				Files.delete(dir);
				return FileVisitResult.CONTINUE;
			}
		});
	}

	public static boolean isGzip(byte[] bytes) {
		return bytes.length >= 2 && (bytes[0] & 0xff) == 0x1f && (bytes[1] & 0xff) == 0x8b;
	}

	public static byte[] gunzip(byte[] bytes) throws IOException {
		try (InputStream in = new GZIPInputStream(new java.io.ByteArrayInputStream(bytes))) {
			ByteArrayOutputStream out = new ByteArrayOutputStream();
			in.transferTo(out);
			return out.toByteArray();
		}
	}

	/**
	 * Upload serialization: byte for byte what the upload DAOs send.
	 */
	// Mirrors dao/connection/ftp/FtpDAO.java:584-588 (SftpDAO.java:318-320, WebdavDAO.java:333-337)
	public static void writeUploadStyle(Object object, Path file) throws IOException {
		Files.createDirectories(file.toAbsolutePath().getParent());
		try (FileOutputStream fos = new FileOutputStream(file.toFile())) {
			ObjectOutputStream oos = new ObjectOutputStream(new GZIPOutputStream(fos));
			oos.writeObject(object);
			oos.flush();
			oos.close();
		}
	}
}
