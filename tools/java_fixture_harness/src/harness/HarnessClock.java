package harness;

import java.util.Date;

/**
 * Pinned build clock. {@code build.sh} patch P1 replaces the three {@code new Date()} calls of
 * {@code dao/repository/RepositoryBuildProcessor.java} (:153, :184, :200) with {@link #now()}.
 */
public final class HarnessClock {

	private static volatile long millis = 0L;

	private HarnessClock() {
	}

	/** Sets the time returned by every following {@link #now()} call. */
	public static void set(long value) {
		millis = value;
	}

	/** Returns a new {@link Date} per call, like the {@code new Date()} it replaces. */
	public static Date now() {
		return new Date(millis);
	}
}
