package harness;

/** A fatal harness error: the command exits with code 3. */
public class HarnessException extends Exception {

	private static final long serialVersionUID = 1L;

	public HarnessException(String message) {
		super(message);
	}

	public HarnessException(String message, Throwable cause) {
		super(message, cause);
	}
}
