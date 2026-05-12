package frida.java.bridge.rs.smoke;

public final class SmokeSubject {
    public static final String STATIC_TEXT = "static-smoke";
    public int number = 7;
    private long hidden = 11L;

    public SmokeSubject() {
    }

    public SmokeSubject(int number) {
        this.number = number;
    }

    public String message() {
        return "dex-smoke";
    }

    public String overload() {
        return "no-args";
    }

    public String overload(String value) {
        return value;
    }

    private static String hiddenStatic() {
        return "hidden";
    }

    public static int answer() {
        return 42;
    }
}
