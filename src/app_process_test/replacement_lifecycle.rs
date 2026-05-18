use super::assertions::*;
use super::replacement_callbacks::*;
use super::*;

pub(super) fn run_replacement_lifecycle_checks(
    java: &Java,
    subject: &JavaClass,
    wrapper: &JavaClassWrapper,
    object: &JavaObject,
) -> Result<()> {
    println!("app_process_test: checking replacement lifecycle replay");

    expect_int(
        subject.call_static("lifecycleStaticAnswer", "()I", &[])?,
        700,
        "lifecycleStaticAnswer original",
    )?;
    let mut replacement = unsafe {
        replacement::replace_static_i32_method(
            subject,
            "lifecycleStaticAnswer",
            replacement_lifecycle_static_a,
        )?
    };
    expect_replacement_clone_backend(&replacement, "lifecycleStaticAnswer first replacement")?;
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
    let mut replacement = unsafe {
        replacement::replace_static_i32_method(
            subject,
            "lifecycleStaticAnswer",
            replacement_lifecycle_static_b,
        )?
    };
    expect_replacement_clone_backend(&replacement, "lifecycleStaticAnswer second replacement")?;
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
    let mut replacement = unsafe {
        replacement::replace_instance_i32_method(
            subject,
            "lifecycleInstanceNumber",
            replacement_lifecycle_instance_a,
        )?
    };
    expect_replacement_clone_backend(&replacement, "lifecycleInstanceNumber first replacement")?;
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
    let mut replacement = unsafe {
        replacement::replace_instance_i32_method(
            subject,
            "lifecycleInstanceNumber",
            replacement_lifecycle_instance_b,
        )?
    };
    expect_replacement_clone_backend(&replacement, "lifecycleInstanceNumber second replacement")?;
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

    let facade_static = wrapper.static_method_overload("facadeLifecycleAnswer", &[])?;
    expect_int(
        facade_static.call_static(())?,
        710,
        "facadeLifecycleAnswer original",
    )?;
    let mut replacement = unsafe {
        facade_static.replace(replacement::MethodImplementation::StaticI32(
            replacement_lifecycle_static_a,
        ))?
    };
    expect_replacement_clone_backend(&replacement, "facadeLifecycleAnswer first replacement")?;
    expect_int(
        facade_static.call_static(())?,
        1700,
        "facadeLifecycleAnswer first replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call_static(())?,
        710,
        "facadeLifecycleAnswer first restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement = unsafe {
        facade_static.replace(replacement::MethodImplementation::StaticI32(
            replacement_lifecycle_static_b,
        ))?
    };
    expect_replacement_clone_backend(&replacement, "facadeLifecycleAnswer second replacement")?;
    expect_int(
        facade_static.call_static(())?,
        2700,
        "facadeLifecycleAnswer second replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call_static(())?,
        710,
        "facadeLifecycleAnswer second restore",
    )?;

    let mut replacement =
        unsafe { facade_static.replace_closure(|_| Ok(replacement::RawJavaReturn::Int(3710)))? };
    let Some(summary) = replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "facadeLifecycleAnswer closure debug summary was unavailable".to_owned(),
        });
    };
    expect_clone_backend_summary(&summary)?;
    expect_int(
        facade_static.call_static(())?,
        3710,
        "facadeLifecycleAnswer first closure replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call_static(())?,
        710,
        "facadeLifecycleAnswer first closure restore",
    )?;
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let mut replacement =
        unsafe { facade_static.replace_closure(|_| Ok(replacement::RawJavaReturn::Int(4710)))? };
    let Some(summary) = replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "facadeLifecycleAnswer second closure debug summary was unavailable".to_owned(),
        });
    };
    expect_clone_backend_summary(&summary)?;
    expect_int(
        facade_static.call_static(())?,
        4710,
        "facadeLifecycleAnswer second closure replacement",
    )?;
    replacement.revert()?;
    expect_int(
        facade_static.call_static(())?,
        710,
        "facadeLifecycleAnswer second closure restore",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    let facade_instance = wrapper.method_overload("facadeLifecycleInstanceNumber", &[])?;
    expect_int(
        facade_instance.call(object, ())?,
        741,
        "facadeLifecycleInstanceNumber original",
    )?;
    let mut replacement = unsafe {
        facade_instance.replace(replacement::MethodImplementation::InstanceI32(
            replacement_lifecycle_instance_a,
        ))?
    };
    expect_replacement_clone_backend(
        &replacement,
        "facadeLifecycleInstanceNumber first replacement",
    )?;
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
    let mut replacement = unsafe {
        facade_instance.replace(replacement::MethodImplementation::InstanceI32(
            replacement_lifecycle_instance_b,
        ))?
    };
    expect_replacement_clone_backend(
        &replacement,
        "facadeLifecycleInstanceNumber second replacement",
    )?;
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
