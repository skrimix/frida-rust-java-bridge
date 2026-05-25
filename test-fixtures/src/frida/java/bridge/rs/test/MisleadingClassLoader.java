package frida.java.bridge.rs.test;

public final class MisleadingClassLoader extends ClassLoader {
    public MisleadingClassLoader() {
        super(MisleadingClassLoader.class.getClassLoader());
    }

    @Override
    public Class<?> loadClass(String name) throws ClassNotFoundException {
        if ("frida.java.bridge.rs.test.TestSubject".equals(name)) {
            return String.class;
        }
        return super.loadClass(name);
    }
}
