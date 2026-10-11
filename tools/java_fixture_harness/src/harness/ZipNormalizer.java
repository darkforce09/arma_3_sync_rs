package harness;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Harness post-processing of {@code .pbo.zip}/{@code .ebo.zip} snapshots: Java writes the current
 * time into the entry ({@code dao/zip/ZipDAO.java:135-136}, JDK {@code putNextEntry}); the snapshot
 * gets DOS time {@code 0x0000} and date {@code 0x0021} (1980-01-01 00:00) in the local header and
 * in the central directory record. Lengths and CRCs are unchanged.
 */
public final class ZipNormalizer {

	private static final int EOCD_SIG = 0x06054b50;
	private static final int CEN_SIG = 0x02014b50;
	private static final int LOC_SIG = 0x04034b50;

	private ZipNormalizer() {
	}

	public static void normalize(Path zip) throws IOException, HarnessException {
		byte[] b = Files.readAllBytes(zip);
		int eocd = b.length - 22;
		if (eocd < 0 || u32(b, eocd) != EOCD_SIG || u16(b, eocd + 20) != 0) {
			throw new HarnessException(zip + ": no comment-less end of central directory record");
		}
		if (u16(b, eocd + 10) != 1) {
			throw new HarnessException(zip + ": expected exactly one entry, got " + u16(b, eocd + 10));
		}
		int cen = (int) u32(b, eocd + 16);
		if (u32(b, cen) != CEN_SIG || u16(b, cen + 30) != 0) {
			throw new HarnessException(zip + ": unexpected central directory record or extra field");
		}
		int loc = (int) u32(b, cen + 42);
		if (u32(b, loc) != LOC_SIG || u16(b, loc + 28) != 0) {
			throw new HarnessException(zip + ": unexpected local header or extra field");
		}
		setDosTime(b, loc + 10);
		setDosTime(b, cen + 12);
		java.nio.file.attribute.FileTime mtime = Files.getLastModifiedTime(zip);
		Files.write(zip, b);
		Files.setLastModifiedTime(zip, mtime);
	}

	private static void setDosTime(byte[] b, int offset) {
		b[offset] = 0x00;
		b[offset + 1] = 0x00;
		b[offset + 2] = 0x21;
		b[offset + 3] = 0x00;
	}

	private static int u16(byte[] b, int offset) {
		return (b[offset] & 0xff) | (b[offset + 1] & 0xff) << 8;
	}

	private static long u32(byte[] b, int offset) {
		return (u16(b, offset) | (long) u16(b, offset + 2) << 16);
	}
}
