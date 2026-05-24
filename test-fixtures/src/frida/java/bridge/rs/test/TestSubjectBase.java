package frida.java.bridge.rs.test;

public class TestSubjectBase {
    public static int inheritedStaticNumber = 61;
    public static int shadowedStaticField = 71;
    public int inheritedNumber = 21;
    public int shadowedNumber = 17;

    public String inheritedMessage() {
        return "base-message";
    }

    public static int inheritedStaticAnswer() {
        return 515;
    }

    public String shadowedMessage() {
        return "base-shadowed";
    }

    public String shadowedMessage(int value) {
        return "base-shadowed-" + value;
    }
}
