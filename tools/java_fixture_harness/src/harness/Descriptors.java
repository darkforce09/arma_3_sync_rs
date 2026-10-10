package harness;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.IOException;
import java.io.ObjectOutputStream;
import java.io.ObjectStreamClass;
import java.io.ObjectStreamConstants;
import java.io.ObjectStreamField;
import java.io.Serializable;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.stream.Stream;

/**
 * Descriptor dump ({@code java_descriptors.txt}): UID, flags, super class, fields, enum constants
 * and transient fields of every serializable class of {@code domain/**} and {@code constant/**},
 * plus the JDK classes that appear in Arma3Sync streams.
 */
public final class Descriptors {

	private static final String[] JDK_CLASSES = { "java.lang.Boolean", "java.lang.Enum", "java.util.ArrayList",
			"java.util.Date", "java.util.HashMap", "java.util.HashSet", "java.util.TreeSet",
			"javax.crypto.SealedObject", "[B" };

	private static final String[] FLAG_NAMES = { "SC_WRITE_METHOD", "SC_SERIALIZABLE", "SC_EXTERNALIZABLE",
			"SC_BLOCK_DATA", "SC_ENUM" };

	private Descriptors() {
	}

	/** Renders the whole descriptor file. */
	public static String render() throws HarnessException {
		List<String> names = new ArrayList<String>();
		Path classes = Main.harnessHome().resolve("build").resolve("classes");
		for (String root : new String[] { "fr/soe/a3s/domain", "fr/soe/a3s/constant" }) {
			names.addAll(scan(classes, root));
		}
		ClassLoader loader = Descriptors.class.getClassLoader();
		List<Class<?>> types = new ArrayList<Class<?>>();
		for (String name : names) {
			Class<?> type = load(name, loader);
			if (type.isInterface() || type.isAnonymousClass() || type.isLocalClass()
					|| !Serializable.class.isAssignableFrom(type)) {
				continue;
			}
			types.add(type);
		}
		for (String name : JDK_CLASSES) {
			types.add(load(name, loader));
		}
		types.sort((a, b) -> a.getName().compareTo(b.getName()));

		StringBuilder sb = new StringBuilder();
		sb.append("# java_fixture_harness descriptors; JDK ").append(System.getProperty("java.runtime.version"))
				.append("; Java commit ").append(Main.javaCommit()).append('\n');
		for (Class<?> type : types) {
			sb.append('\n');
			describe(sb, type);
		}
		return sb.toString();
	}

	private static Class<?> load(String name, ClassLoader loader) throws HarnessException {
		try {
			return Class.forName(name, false, loader);
		} catch (ClassNotFoundException e) {
			throw new HarnessException("cannot load " + name, e);
		}
	}

	private static List<String> scan(Path classes, String root) throws HarnessException {
		Path dir = classes.resolve(root);
		List<String> names = new ArrayList<String>();
		try (Stream<Path> paths = Files.walk(dir)) {
			paths.filter(p -> p.getFileName().toString().endsWith(".class")).forEach(p -> {
				String rel = classes.relativize(p).toString().replace('\\', '/');
				String name = rel.substring(0, rel.length() - ".class".length()).replace('/', '.');
				if (!name.endsWith("package-info") && !name.endsWith("module-info")) {
					names.add(name);
				}
			});
		} catch (IOException e) {
			throw new HarnessException("cannot scan " + dir, e);
		}
		Collections.sort(names);
		return names;
	}

	private static void describe(StringBuilder sb, Class<?> type) throws HarnessException {
		long[] uidAndFlags = wireDescriptor(type);
		sb.append("class ").append(type.getName()).append('\n');
		sb.append("  uid ").append(uidAndFlags[0]).append(hasDeclaredUid(type) ? " declared" : " computed")
				.append('\n');
		int flags = (int) uidAndFlags[1];
		sb.append("  flags ").append(String.format("0x%02x", flags));
		List<String> flagNames = new ArrayList<String>();
		for (int bit = 0; bit < FLAG_NAMES.length; bit++) {
			if ((flags & (1 << bit)) != 0) {
				flagNames.add(FLAG_NAMES[bit]);
			}
		}
		if (!flagNames.isEmpty()) {
			sb.append(' ').append(String.join("|", flagNames));
		}
		sb.append('\n');
		sb.append("  super ").append(serializableSuper(type)).append('\n');
		ObjectStreamClass osc = ObjectStreamClass.lookupAny(type);
		for (ObjectStreamField field : osc.getFields()) {
			sb.append("  field ").append(field.getTypeCode()).append(' ').append(field.getName());
			if (field.getTypeString() != null) {
				sb.append(' ').append(field.getTypeString());
			}
			sb.append('\n');
		}
		if (type.isEnum()) {
			List<String> constants = new ArrayList<String>();
			for (Object constant : type.getEnumConstants()) {
				constants.add(((Enum<?>) constant).name());
			}
			if (!constants.isEmpty()) {
				sb.append("  enum ").append(String.join(",", constants)).append('\n');
			}
		}
		List<String> transients = new ArrayList<String>();
		for (Field field : type.getDeclaredFields()) {
			int mods = field.getModifiers();
			if (Modifier.isTransient(mods) && !Modifier.isStatic(mods)) {
				transients.add(field.getName());
			}
		}
		Collections.sort(transients);
		if (!transients.isEmpty()) {
			sb.append("  transient ").append(String.join(",", transients)).append('\n');
		}
	}

	/** Writes the {@code Class} object and parses the first {@code TC_CLASSDESC}: UID and flags. */
	private static long[] wireDescriptor(Class<?> type) throws HarnessException {
		try {
			ByteArrayOutputStream bytes = new ByteArrayOutputStream();
			try (ObjectOutputStream out = new ObjectOutputStream(bytes)) {
				out.writeObject(type);
			}
			DataInputStream in = new DataInputStream(new ByteArrayInputStream(bytes.toByteArray()));
			in.readInt(); // magic + version
			if (in.readByte() != ObjectStreamConstants.TC_CLASS
					|| in.readByte() != ObjectStreamConstants.TC_CLASSDESC) {
				throw new HarnessException("unexpected stream for " + type.getName());
			}
			String name = in.readUTF();
			if (!name.equals(type.getName())) {
				throw new HarnessException("descriptor name " + name + " for " + type.getName());
			}
			long uid = in.readLong();
			int flags = in.readByte() & 0xff;
			return new long[] { uid, flags };
		} catch (IOException e) {
			throw new HarnessException("cannot serialize " + type.getName(), e);
		}
	}

	private static boolean hasDeclaredUid(Class<?> type) {
		try {
			Field field = type.getDeclaredField("serialVersionUID");
			int mods = field.getModifiers();
			return Modifier.isStatic(mods) && Modifier.isFinal(mods) && field.getType() == long.class;
		} catch (NoSuchFieldException e) {
			return false;
		}
	}

	private static String serializableSuper(Class<?> type) {
		for (Class<?> c = type.getSuperclass(); c != null; c = c.getSuperclass()) {
			if (Serializable.class.isAssignableFrom(c)) {
				return c.getName();
			}
		}
		return "-";
	}
}
