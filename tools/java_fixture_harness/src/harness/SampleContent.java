package harness;

import java.nio.charset.StandardCharsets;

/**
 * Deterministic sample file content: the text line {@code "A3S-SAMPLE <path>\n"} followed by
 * xorshift32 bytes seeded from the path's {@code String.hashCode()} (and a generation number for
 * files rewritten by a later step), truncated to the requested size.
 */
public final class SampleContent {

	private SampleContent() {
	}

	public static byte[] bytes(String path, int size, int generation) {
		byte[] out = new byte[size];
		byte[] header = ("A3S-SAMPLE " + path + "\n").getBytes(StandardCharsets.UTF_8);
		int n = Math.min(size, header.length);
		System.arraycopy(header, 0, out, 0, n);
		int x = path.hashCode() ^ (generation * 0x9E3779B9);
		if (x == 0) {
			x = 0x2545F491;
		}
		for (int i = n; i < size; i++) {
			x ^= x << 13;
			x ^= x >>> 17;
			x ^= x << 5;
			out[i] = (byte) (x >>> 24);
		}
		return out;
	}
}
