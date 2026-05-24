package frida.java.bridge.rs.test;

public class TestSubjectBase {
    public int inheritedNumber = 21;
    public int shadowedNumber = 17;

    public String inheritedMessage() {
        return "base-message";
    }
}
