package harness;

import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Date;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import fr.soe.a3s.dao.A3SFilesAccessor;
import fr.soe.a3s.domain.repository.Event;
import fr.soe.a3s.domain.repository.Events;
import fr.soe.a3s.domain.repository.ServerInfo;

/**
 * JDK collection round trips ({@code java_roundtrip/}): {@code HashSet}/{@code HashMap}
 * capacities written by fresh objects and after read + upload-style re-serialization.
 */
public final class RoundtripFixtures {

	static final int[] SET_SIZES = { 0, 1, 2, 4, 7, 12, 13, 24, 25 };
	static final int[] MAP_SIZES = { 1, 6, 12, 13, 24 };

	private RoundtripFixtures() {
	}

	static void generate(Path out) throws Exception {
		Path root = out.resolve("java_roundtrip");
		List<String> lines = new ArrayList<String>();
		for (int n : SET_SIZES) {
			String name = "serverinfo_hidden_" + n;
			Path dir = root.resolve(name);
			Files.createDirectories(dir);
			// Built as in dao/repository/RepositoryBuildProcessor.java:149-170
			ServerInfo serverInfo = new ServerInfo();
			serverInfo.setRevision(1);
			serverInfo.setBuildDate(new Date(BuiltFixtures.BUILD_DATES[0]));
			serverInfo.setNumberOfFiles(n);
			serverInfo.setTotalFilesSize(1000L * n);
			serverInfo.setNumberOfConnections(1);
			serverInfo.setNoPartialFileTransfer(false);
			serverInfo.setRepositoryContentUpdated(true);
			for (int i = 0; i < n; i++) {
				serverInfo.getHiddenFolderPaths().add(String.format("h%02d", i));
			}
			A3SFilesAccessor.write(serverInfo, dir.resolve("fresh").toFile());
			roundTrip(dir.resolve("fresh"), dir.resolve("rt1"));
			roundTrip(dir.resolve("rt1"), dir.resolve("rt2"));
			for (String file : List.of("fresh", "rt1", "rt2")) {
				lines.add(capacityLine(name, file, dir.resolve(file), "java.util.HashSet"));
			}
		}
		for (int n : MAP_SIZES) {
			String name = "events_map_" + n;
			Path dir = root.resolve(name);
			Files.createDirectories(dir);
			Map<String, Boolean> source = new HashMap<String, Boolean>();
			Events put = new Events();
			Event putEvent = new Event("E");
			put.getList().add(putEvent);
			for (int i = 0; i < n; i++) {
				String key = String.format("k%02d", i);
				putEvent.getAddonNames().put(key, Boolean.TRUE);
				source.put(key, Boolean.TRUE);
			}
			Events putAll = new Events();
			Event putAllEvent = new Event("E");
			putAll.getList().add(putAllEvent);
			putAllEvent.getAddonNames().putAll(source);
			A3SFilesAccessor.write(put, dir.resolve("fresh_put").toFile());
			A3SFilesAccessor.write(putAll, dir.resolve("fresh_putall").toFile());
			roundTrip(dir.resolve("fresh_put"), dir.resolve("rt1"));
			for (String file : List.of("fresh_put", "fresh_putall", "rt1")) {
				lines.add(capacityLine(name, file, dir.resolve(file), "java.util.HashMap"));
			}
		}
		Collections.sort(lines);
		Io.writeString(root.resolve("capacities.txt"), String.join("", lines));
	}

	/** {@code A3SFilesAccessor.read} followed by the upload serialization. */
	private static void roundTrip(Path in, Path out) throws IOException {
		Io.writeUploadStyle(A3SFilesAccessor.read(in.toFile()), out);
	}

	private static String capacityLine(String name, String file, Path path, String className)
			throws IOException, HarnessException {
		int[] values = firstBlock(Io.gunzip(Files.readAllBytes(path)), className);
		return name + " " + file + " capacity=" + values[0] + " size=" + values[1] + "\n";
	}

	/**
	 * Reads the first {@code TC_BLOCKDATA} after the class descriptor name: {@code int capacity,
	 * float loadFactor, int size} for {@code HashSet}, {@code int buckets, int size} for
	 * {@code HashMap}.
	 */
	static int[] firstBlock(byte[] stream, String className) throws IOException, HarnessException {
		byte[] name = className.getBytes(StandardCharsets.UTF_8);
		int at = indexOf(stream, name, 0);
		if (at < 0) {
			throw new HarnessException("no " + className + " descriptor in the stream");
		}
		int length = className.equals("java.util.HashSet") ? 12 : 8;
		for (int i = at + name.length; i + 1 < stream.length; i++) {
			if (stream[i] == 0x77 && (stream[i + 1] & 0xff) == length) {
				DataInputStream in = new DataInputStream(new ByteArrayInputStream(stream, i + 2, length));
				int capacity = in.readInt();
				if (length == 12) {
					in.readFloat();
				}
				return new int[] { capacity, in.readInt() };
			}
		}
		throw new HarnessException("no block data after the " + className + " descriptor");
	}

	private static int indexOf(byte[] haystack, byte[] needle, int from) {
		outer: for (int i = from; i + needle.length <= haystack.length; i++) {
			for (int j = 0; j < needle.length; j++) {
				if (haystack[i + j] != needle[j]) {
					continue outer;
				}
			}
			return i;
		}
		return -1;
	}
}
