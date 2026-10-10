package harness;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;

/** {@code java_harness_manifest.txt}: header plus one {@code <sha256>  <size>  <path>} line per file. */
public final class Manifest {

	static final String FILE_NAME = "java_harness_manifest.txt";

	private Manifest() {
	}

	public static void write(Path out) throws IOException, HarnessException {
		StringBuilder sb = new StringBuilder();
		sb.append("# java_fixture_harness manifest\n");
		sb.append("jdk ").append(System.getProperty("java.runtime.version")).append(' ')
				.append(System.getProperty("java.vendor")).append('\n');
		sb.append("java_commit ").append(Main.javaCommit()).append('\n');
		sb.append("harness_version 1\n");
		long total = 0;
		for (String rel : Io.files(out)) {
			if (rel.equals(FILE_NAME)) {
				continue;
			}
			byte[] bytes = Files.readAllBytes(out.resolve(rel));
			total += bytes.length;
			sb.append(sha256(bytes)).append("  ").append(bytes.length).append("  ").append(rel).append('\n');
		}
		if (total > Main.MAX_TOTAL_BYTES) {
			throw new HarnessException("generated fixtures total " + total + " bytes, over the "
					+ Main.MAX_TOTAL_BYTES + " byte cap");
		}
		Io.writeString(out.resolve(FILE_NAME), sb.toString());
		System.err.println("manifest: " + total + " bytes");
	}

	static String sha256(byte[] bytes) {
		try {
			return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
		} catch (NoSuchAlgorithmException e) {
			throw new IllegalStateException(e);
		}
	}
}
