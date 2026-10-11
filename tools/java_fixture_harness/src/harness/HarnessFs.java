package harness;

import java.io.File;
import java.util.Arrays;
import java.util.Comparator;

/**
 * Deterministic directory listing. {@code build.sh} patch P2 wraps the two {@code listFiles()}
 * calls of {@code dao/repository/RepositoryBuildProcessor.java} (:116, :475) with
 * {@link #sorted(File[])}, so the scan order is the sorted order the Rust builder uses.
 */
public final class HarnessFs {

	private HarnessFs() {
	}

	/** Sorts in place by {@link File#getName()} with {@link String#compareTo}; tolerates null. */
	public static File[] sorted(File[] files) {
		if (files != null) {
			Arrays.sort(files, Comparator.comparing(File::getName));
		}
		return files;
	}
}
