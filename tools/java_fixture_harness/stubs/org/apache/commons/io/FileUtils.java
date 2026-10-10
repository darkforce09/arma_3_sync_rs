package org.apache.commons.io;

import java.io.File;
import java.util.Objects;

/**
 * Harness stub of commons-io 2.15 {@code FileUtils}. Only {@link #sizeOf(File)} is reachable from
 * the Arma3Sync classes the harness compiles ({@code RepositoryBuildProcessor.java:494}); its
 * behaviour mirrors the bytecode of {@code resources/lib/commons-io-2.15.0.jar}.
 */
public final class FileUtils {

	private FileUtils() {
	}

	/**
	 * Returns the size of a file, or the recursive sum of the regular-file sizes of a directory.
	 */
	public static long sizeOf(File file) {
		Objects.requireNonNull(file, "file");
		if (!file.exists()) {
			throw new IllegalArgumentException(
					"File system element for parameter 'file' does not exist: '" + file + "'");
		}
		if (file.isDirectory()) {
			return sizeOfDirectory(file);
		}
		return file.length();
	}

	private static long sizeOfDirectory(File directory) {
		File[] children = directory.listFiles();
		if (children == null) {
			return 0L;
		}
		long size = 0L;
		for (File child : children) {
			if (child.isDirectory()) {
				size += sizeOfDirectory(child);
			} else if (child.isFile()) {
				size += child.length();
			}
		}
		return size;
	}
}
