use super::assertions::*;
use super::*;

pub(super) fn run_replacement_lifecycle_checks(
    java: &Java,
    subject: &raw::Class,
    wrapper: &JavaClass,
    object: &JavaObject,
) -> Result<()> {
    println!("app_process_test: checking replacement lifecycle replay");

    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        700,
        "lifecycleStaticAnswer original",
    )?;
    let lifecycle_static =
        JavaMethod::from_raw_exact(subject, MethodKind::Static, "lifecycleStaticAnswer", "()I")?;
    let mut replacement = lifecycle_static.replace(|ctx| ctx.ret(1700))?;
    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        1700,
        "lifecycleStaticAnswer first replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        700,
        "lifecycleStaticAnswer first restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = lifecycle_static.replace(|ctx| ctx.ret(2700))?;
    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        2700,
        "lifecycleStaticAnswer second replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        700,
        "lifecycleStaticAnswer second restore",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    expect_int(
        subject.call_method(object, "lifecycleInstanceNumber", "()I", &[])?,
        731,
        "lifecycleInstanceNumber original",
    )?;
    let lifecycle_instance = JavaMethod::from_raw_exact(
        subject,
        MethodKind::Instance,
        "lifecycleInstanceNumber",
        "()I",
    )?;
    let mut replacement = lifecycle_instance.replace(|ctx| ctx.ret(1701))?;
    expect_int(
        subject.call_method(object, "lifecycleInstanceNumber", "()I", &[])?,
        1701,
        "lifecycleInstanceNumber first replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(object, "lifecycleInstanceNumber", "()I", &[])?,
        731,
        "lifecycleInstanceNumber first restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = lifecycle_instance.replace(|ctx| ctx.ret(2701))?;
    expect_int(
        subject.call_method(object, "lifecycleInstanceNumber", "()I", &[])?,
        2701,
        "lifecycleInstanceNumber second replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(object, "lifecycleInstanceNumber", "()I", &[])?,
        731,
        "lifecycleInstanceNumber second restore",
    )?;

    let facade_static = wrapper
        .method("facadeLifecycleAnswer")?
        .overload([] as [&str; 0])?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer original",
    )?;
    let mut replacement = facade_static.replace(|ctx| ctx.ret(1700))?;
    expect_int(
        facade_static.call((), ())?,
        1700,
        "facadeLifecycleAnswer first replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer first restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = facade_static.replace(|ctx| ctx.ret(2700))?;
    expect_int(
        facade_static.call((), ())?,
        2700,
        "facadeLifecycleAnswer second replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer second restore",
    )?;

    let mut replacement = facade_static.replace(|ctx| ctx.ret(3710))?;
    expect_int(
        facade_static.call((), ())?,
        3710,
        "facadeLifecycleAnswer first closure replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer first closure restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = facade_static.replace(|ctx| ctx.ret(4710))?;
    expect_int(
        facade_static.call((), ())?,
        4710,
        "facadeLifecycleAnswer second closure replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer second closure restore",
    )?;

    let entered = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let entered_callback = Arc::clone(&entered);
    let release_callback = Arc::clone(&release);
    let mut replacement = facade_static.replace(move |ctx| {
        let (entered, entered_cvar) = &*entered_callback;
        *entered
            .lock()
            .expect("replacement lifecycle entered mutex poisoned") = true;
        entered_cvar.notify_all();

        let (release, release_cvar) = &*release_callback;
        let mut released = release
            .lock()
            .expect("replacement lifecycle release mutex poisoned");
        while !*released {
            released = release_cvar
                .wait(released)
                .expect("replacement lifecycle release mutex poisoned");
        }

        ctx.ret(5710)
    })?;
    let threaded_facade_static = facade_static.clone();
    let worker = std::thread::spawn(move || threaded_facade_static.call::<jni::jint>((), ()));

    let (entered, entered_cvar) = &*entered;
    let entered_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut did_enter = entered
        .lock()
        .expect("replacement lifecycle entered mutex poisoned");
    while !*did_enter {
        let now = std::time::Instant::now();
        if now >= entered_deadline {
            return Err(Error::UnsupportedFeature {
                feature: "method replacement lifecycle",
                reason: "threaded replacement callback did not enter before timeout".to_owned(),
            });
        }
        let (guard, _) = entered_cvar
            .wait_timeout(did_enter, entered_deadline - now)
            .expect("replacement lifecycle entered mutex poisoned");
        did_enter = guard;
    }
    drop(did_enter);

    let release_after_delay = Arc::clone(&release);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let (release, release_cvar) = &*release_after_delay;
        *release
            .lock()
            .expect("replacement lifecycle release mutex poisoned") = true;
        release_cvar.notify_all();
    });
    let revert_started = std::time::Instant::now();
    replacement.revert()?;
    if revert_started.elapsed() < std::time::Duration::from_millis(50) {
        return Err(Error::UnsupportedFeature {
            feature: "method replacement lifecycle",
            reason: "replacement revert returned before active callback drained".to_owned(),
        });
    }
    let threaded_result = worker.join().map_err(|_| Error::UnsupportedFeature {
        feature: "method replacement lifecycle",
        reason: "threaded replacement caller panicked".to_owned(),
    })??;
    expect_int(
        JavaReturn::Int(threaded_result),
        5710,
        "facadeLifecycleAnswer threaded replacement result",
    )?;
    releaser.join().map_err(|_| Error::UnsupportedFeature {
        feature: "method replacement lifecycle",
        reason: "replacement callback releaser panicked".to_owned(),
    })?;
    expect_int(
        facade_static.call((), ())?,
        710,
        "facadeLifecycleAnswer threaded replacement restore",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    let facade_instance = wrapper
        .method("facadeLifecycleInstanceNumber")?
        .overload([] as [&str; 0])?;
    expect_int(
        facade_instance.call(object, ())?,
        741,
        "facadeLifecycleInstanceNumber original",
    )?;
    let mut replacement = facade_instance.replace(|ctx| ctx.ret(1701))?;
    expect_int(
        facade_instance.call(object, ())?,
        1701,
        "facadeLifecycleInstanceNumber first replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_instance.call(object, ())?,
        741,
        "facadeLifecycleInstanceNumber first restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = facade_instance.replace(|ctx| ctx.ret(2701))?;
    expect_int(
        facade_instance.call(object, ())?,
        2701,
        "facadeLifecycleInstanceNumber second replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_instance.call(object, ())?,
        741,
        "facadeLifecycleInstanceNumber second restore",
    )?;

    EXPECTED_RECEIVER.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}
