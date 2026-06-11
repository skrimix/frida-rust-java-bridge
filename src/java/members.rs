use std::fmt;

use crate::{
    env::{FieldKind, MethodKind},
    error::{Error, Result},
    jni,
    metadata::{self, JavaFieldMetadata, JavaMethodMetadata},
    refs::{AsJClass, JavaObjectRef},
    signature::{JavaType, MethodSignature},
    value::JavaValue,
};

use super::{
    AttachedJavaCallArgs, FromJavaReturn, IntoJavaCallArgs, IntoJavaFieldValue, Java,
    JavaOverloadArg, JavaReturn, JavaReturnRef,
    array::JavaArray,
    class::JavaClass,
    dispatch::RawObject,
    object::{JavaBoundFieldHandle, JavaBoundMethodGroup, JavaBoundMethodOverload, JavaObject},
    raw,
};

/// A named Java method group containing the currently visible non-constructor overloads.
#[derive(Clone)]
pub struct JavaMethodGroup {
    pub(super) class: raw::Class,
    pub(super) name: String,
    pub(super) overloads: Vec<JavaMethodMetadata>,
}

/// A selected constructor overload on a `JavaClass`.
#[derive(Clone)]
pub struct JavaConstructor {
    pub(super) class: raw::Class,
    pub(super) metadata: JavaMethodMetadata,
}

/// A selected method on a `JavaClass`.
#[derive(Clone)]
pub struct JavaMethod {
    pub(super) class: raw::Class,
    pub(super) metadata: JavaMethodMetadata,
}

/// A selected field on a `JavaClass`.
#[derive(Clone)]
pub struct JavaField {
    pub(super) class: raw::Class,
    pub(super) metadata: JavaFieldMetadata,
}

impl fmt::Debug for JavaMethodGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethodGroup")
            .field("class", &self.class.name())
            .field("name", &self.name)
            .field("overloads", &self.overloads)
            .finish()
    }
}

impl fmt::Display for JavaConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "function {}.<init>{}",
            self.class.name(),
            self.signature()
        )
    }
}

impl fmt::Debug for JavaConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaConstructor")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaConstructor {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for JavaMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "function {}.{}{}",
            self.class.name(),
            self.name(),
            self.signature()
        )
    }
}

impl fmt::Debug for JavaMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethod")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaMethod {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for JavaField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "field {}.{}: {}",
            self.class.name(),
            self.name(),
            self.ty()
        )
    }
}

impl fmt::Debug for JavaField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaField")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaField {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Debug for JavaBoundMethodGroup<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundMethodGroup")
            .field("object", &self.object.as_jobject())
            .field("group", &self.group)
            .finish()
    }
}

impl fmt::Debug for JavaBoundMethodOverload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundMethodOverload")
            .field("object", &self.object.as_jobject())
            .field("overload", &self.overload)
            .finish()
    }
}

impl fmt::Debug for JavaBoundFieldHandle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundFieldHandle")
            .field("object", &self.object.as_jobject())
            .field("field", &self.field)
            .finish()
    }
}

impl JavaMethodGroup {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        &self.overloads
    }

    pub fn overload<'types>(&self, arguments: impl AsRef<[&'types str]>) -> Result<JavaMethod> {
        let arguments = parse_type_names(arguments.as_ref())?;
        self.overload_by_types(&arguments)
    }

    pub fn overload_by_types(&self, arguments: &[JavaType]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_group_by_arguments(
                self.class.name(),
                &self.name,
                arguments,
                self.overloads.clone(),
            )?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        let args = args.into_java_overload_args();
        self.dispatch_static(&args)?.call((), args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.overload(arguments)?.call((), args)
    }

    pub(crate) fn unambiguous(&self) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_group_by_name(
                self.class.name(),
                &self.name,
                self.overloads.clone(),
            )?,
        })
    }

    fn dispatch_static(&self, args: &[JavaOverloadArg]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_by_dispatch_args(
                &self.class,
                MethodDispatchTarget::StaticMethod,
                &self.name,
                args,
                self.overloads.clone(),
            )?,
        })
    }

    pub(super) fn dispatch_bound(&self, args: &[JavaOverloadArg]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_by_dispatch_args(
                &self.class,
                MethodDispatchTarget::BoundMethod,
                &self.name,
                args,
                self.overloads.clone(),
            )?,
        })
    }
}

impl JavaConstructor {
    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &raw::Class {
        &self.class
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    /// Requests ART deoptimization for this selected constructor overload.
    ///
    /// The operation is process-runtime state, so it succeeds only when the current ART
    /// backend reports deoptimization support.
    pub fn deoptimize(&self) -> Result<()> {
        self.class
            .vm()
            .art()
            .deoptimize_method(self.class.vm(), self.metadata.id)
    }

    pub fn new_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args =
            AttachedJavaCallArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        validate_reference_call_args(
            &self.class,
            self.metadata.signature.arguments(),
            args.values(),
        )?;
        self.class
            .new_object(&self.metadata.signature.to_string(), args.values())
    }
}

impl JavaMethod {
    pub(crate) fn from_raw_exact(
        class: &raw::Class,
        kind: MethodKind,
        name: &str,
        signature: &str,
    ) -> Result<Self> {
        if kind == MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::from_raw_exact",
            });
        }

        let signature = MethodSignature::parse(signature)?;
        let normalized = signature.to_string();
        let method = match kind {
            MethodKind::Static => class.resolve_static_method(name, &normalized)?,
            MethodKind::Instance => class.resolve_instance_method(name, &normalized)?,
            MethodKind::Constructor => unreachable!("constructor was rejected above"),
        };

        Ok(Self {
            class: class.clone(),
            metadata: JavaMethodMetadata {
                name: name.to_owned(),
                kind,
                signature,
                modifiers: 0,
                id: unsafe { method.raw() },
            },
        })
    }

    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &raw::Class {
        &self.class
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn kind(&self) -> MethodKind {
        self.metadata.kind
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    /// Requests ART deoptimization for this selected method overload.
    pub fn deoptimize(&self) -> Result<()> {
        self.class
            .vm()
            .art()
            .deoptimize_method(self.class.vm(), self.metadata.id)
    }

    pub fn call_raw<A: IntoJavaCallArgs>(
        &self,
        receiver: impl JavaMethodReceiver,
        args: A,
    ) -> Result<JavaReturn> {
        receiver.call(self, args)
    }

    pub fn call<T: FromJavaReturn>(
        &self,
        receiver: impl JavaMethodReceiver,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        T::from_java_return(
            self.bind_declared_return(self.call_raw(receiver, args)?)?,
            "JavaMethod::call",
        )
    }

    pub fn call_void<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<()> {
        self.call_raw(object, args)?
            .into_void("JavaMethod::call_void")
    }

    pub fn call_boolean<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<bool> {
        self.call_raw(object, args)?
            .into_boolean("JavaMethod::call_boolean")
    }

    pub fn call_int<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<jni::jint> {
        self.call_raw(object, args)?
            .into_int("JavaMethod::call_int")
    }

    pub fn call_object<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<JavaObject>> {
        self.bind_declared_return(self.call_raw(object, args)?)?
            .into_object("JavaMethod::call_object")
    }

    pub fn call_array<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<JavaArray>> {
        self.call_raw(object, args)?
            .into_array("JavaMethod::call_array")
    }

    pub fn call_string<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<String>> {
        self.call_object(object, args)?
            .map(|object| object.get_string())
            .transpose()
    }
}

impl JavaMethod {
    pub(super) fn bind_declared_return(&self, value: JavaReturn) -> Result<JavaReturn> {
        bind_declared_return(
            &self.class,
            self.metadata.signature.return_type(),
            value,
            "JavaMethod::call",
        )
    }
}

pub trait JavaMethodReceiver {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn>;
}

impl JavaMethodReceiver for () {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn> {
        if method.metadata.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::call",
            });
        }
        let args = AttachedJavaCallArgs::new(
            method.class.vm(),
            method.metadata.signature.arguments(),
            args,
        )?;
        validate_reference_call_args(
            &method.class,
            method.metadata.signature.arguments(),
            args.values(),
        )?;
        method.class.call_static(
            &method.metadata.name,
            &method.metadata.signature.to_string(),
            args.values(),
        )
    }
}

impl<T: JavaObjectRef + ?Sized> JavaMethodReceiver for &T {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn> {
        match method.metadata.kind {
            MethodKind::Instance => {
                validate_selected_receiver(&method.class, *self, "JavaMethod::call receiver")?;
                let args = AttachedJavaCallArgs::new(
                    method.class.vm(),
                    method.metadata.signature.arguments(),
                    args,
                )?;
                validate_reference_call_args(
                    &method.class,
                    method.metadata.signature.arguments(),
                    args.values(),
                )?;
                method.class.call_method(
                    *self,
                    &method.metadata.name,
                    &method.metadata.signature.to_string(),
                    args.values(),
                )
            }
            MethodKind::Static => {
                let args = AttachedJavaCallArgs::new(
                    method.class.vm(),
                    method.metadata.signature.arguments(),
                    args,
                )?;
                validate_reference_call_args(
                    &method.class,
                    method.metadata.signature.arguments(),
                    args.values(),
                )?;
                method.class.call_static(
                    &method.metadata.name,
                    &method.metadata.signature.to_string(),
                    args.values(),
                )
            }
            MethodKind::Constructor => Err(Error::WrongMethodKind {
                operation: "JavaMethod::call",
            }),
        }
    }
}

impl JavaField {
    pub fn metadata(&self) -> &JavaFieldMetadata {
        &self.metadata
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn kind(&self) -> FieldKind {
        self.metadata.kind
    }

    pub fn ty(&self) -> &JavaType {
        &self.metadata.ty
    }

    pub fn get_raw(&self, receiver: impl JavaFieldReceiver) -> Result<JavaReturn> {
        receiver.get(self)
    }

    pub fn get<T: FromJavaReturn>(&self, receiver: impl JavaFieldReceiver) -> Result<T> {
        T::from_java_return(
            self.bind_declared_return(self.get_raw(receiver)?)?,
            "JavaField::get",
        )
    }

    pub fn get_boolean(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.get_raw(object)?.into_boolean("JavaField::get_boolean")
    }

    pub fn get_byte(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jbyte> {
        self.get_raw(object)?.into_byte("JavaField::get_byte")
    }

    pub fn get_char(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jchar> {
        self.get_raw(object)?.into_char("JavaField::get_char")
    }

    pub fn get_short(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jshort> {
        self.get_raw(object)?.into_short("JavaField::get_short")
    }

    pub fn get_int(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jint> {
        self.get_raw(object)?.into_int("JavaField::get_int")
    }

    pub fn get_long(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jlong> {
        self.get_raw(object)?.into_long("JavaField::get_long")
    }

    pub fn get_float(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jfloat> {
        self.get_raw(object)?.into_float("JavaField::get_float")
    }

    pub fn get_double(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jdouble> {
        self.get_raw(object)?.into_double("JavaField::get_double")
    }

    pub fn get_object(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaObject>> {
        self.bind_declared_return(self.get_raw(object)?)?
            .into_object("JavaField::get_object")
    }

    pub fn get_array(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaArray>> {
        self.get_raw(object)?.into_array("JavaField::get_array")
    }

    pub fn set<V: IntoJavaFieldValue>(
        &self,
        receiver: impl JavaFieldReceiver,
        value: V,
    ) -> Result<()> {
        receiver.set(self, value)
    }

    pub fn set_boolean(&self, object: &(impl JavaObjectRef + ?Sized), value: bool) -> Result<()> {
        self.set(object, JavaValue::Boolean(value))
    }

    pub fn set_byte(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jbyte,
    ) -> Result<()> {
        self.set(object, JavaValue::Byte(value))
    }

    pub fn set_char(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jchar,
    ) -> Result<()> {
        self.set(object, JavaValue::Char(value))
    }

    pub fn set_short(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jshort,
    ) -> Result<()> {
        self.set(object, JavaValue::Short(value))
    }

    pub fn set_int(&self, object: &(impl JavaObjectRef + ?Sized), value: jni::jint) -> Result<()> {
        self.set(object, JavaValue::Int(value))
    }

    pub fn set_long(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jlong,
    ) -> Result<()> {
        self.set(object, JavaValue::Long(value))
    }

    pub fn set_float(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jfloat,
    ) -> Result<()> {
        self.set(object, JavaValue::Float(value))
    }

    pub fn set_double(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jdouble,
    ) -> Result<()> {
        self.set(object, JavaValue::Double(value))
    }

    pub fn set_object<T: JavaObjectRef + ?Sized>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: Option<&T>,
    ) -> Result<()> {
        self.set(
            object,
            value.map_or(JavaValue::NULL, |value| {
                JavaValue::object_ref(value.as_jobject())
            }),
        )
    }
}

impl JavaField {
    pub(super) fn bind_declared_return(&self, value: JavaReturn) -> Result<JavaReturn> {
        bind_declared_return(&self.class, &self.metadata.ty, value, "JavaField::get")
    }
}

pub trait JavaFieldReceiver {
    fn get(&self, field: &JavaField) -> Result<JavaReturn>;
    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()>;
}

impl JavaFieldReceiver for () {
    fn get(&self, field: &JavaField) -> Result<JavaReturn> {
        if field.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get",
            });
        }
        field
            .class
            .get_static_field(&field.metadata.name, &field.metadata.ty.to_string())
    }

    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()> {
        if field.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set",
            });
        }
        let env = field.class.vm().attach_current_thread()?;
        let value = value.into_java_field_value(&env, &field.metadata.ty, "JavaField::set")?;
        let result = validate_reference_field_value(
            &field.class,
            &field.metadata.ty,
            value.value(),
            "JavaField::set",
        )
        .and_then(|()| {
            field.class.set_static_field(
                &field.metadata.name,
                &field.metadata.ty.to_string(),
                value.value(),
            )
        });
        value.delete_local_ref(&env);
        result
    }
}

impl<T: JavaObjectRef + ?Sized> JavaFieldReceiver for &T {
    fn get(&self, field: &JavaField) -> Result<JavaReturn> {
        if field.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get",
            });
        }
        validate_selected_receiver(&field.class, *self, "JavaField::get receiver")?;
        field
            .class
            .get_field(*self, &field.metadata.name, &field.metadata.ty.to_string())
    }

    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()> {
        if field.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set",
            });
        }
        validate_selected_receiver(&field.class, *self, "JavaField::set receiver")?;
        let env = field.class.vm().attach_current_thread()?;
        let value = value.into_java_field_value(&env, &field.metadata.ty, "JavaField::set")?;
        let result = validate_reference_field_value(
            &field.class,
            &field.metadata.ty,
            value.value(),
            "JavaField::set",
        )
        .and_then(|()| {
            field.class.set_field(
                *self,
                &field.metadata.name,
                &field.metadata.ty.to_string(),
                value.value(),
            )
        });
        value.delete_local_ref(&env);
        result
    }
}

#[derive(Clone, Copy)]
pub(super) enum MethodDispatchTarget {
    Constructor,
    StaticMethod,
    BoundMethod,
}

pub(super) fn select_method_by_dispatch_args(
    holder: &raw::Class,
    target: MethodDispatchTarget,
    name: &str,
    args: &[JavaOverloadArg],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let mut candidates = Vec::new();
    let mut best: Option<(i32, usize, JavaMethodMetadata)> = None;

    for (index, method) in methods.into_iter().enumerate() {
        if !dispatch_target_accepts(target, name, &method) {
            continue;
        }

        candidates.push(format!(
            "{} {}",
            method_kind_name(method.kind),
            method.signature
        ));

        let Some(score) = dispatch_score(holder, args, method.signature.arguments())? else {
            continue;
        };

        if best
            .as_ref()
            .is_none_or(|(best_score, best_index, _)| (score, index) < (*best_score, *best_index))
        {
            best = Some((score, index, method));
        }
    }

    best.map(|(_, _, method)| method)
        .ok_or_else(|| Error::NoCompatibleOverload {
            class: holder.name().to_owned(),
            kind: dispatch_target_kind_name(target),
            name: dispatch_target_method_name(target, name).to_owned(),
            arguments: format_dispatch_argument_list(args),
            candidates,
        })
}

fn dispatch_target_accepts(
    target: MethodDispatchTarget,
    name: &str,
    method: &JavaMethodMetadata,
) -> bool {
    match target {
        MethodDispatchTarget::Constructor => method.kind == MethodKind::Constructor,
        MethodDispatchTarget::StaticMethod => {
            method.kind == MethodKind::Static && method.name == name
        }
        MethodDispatchTarget::BoundMethod => {
            method.kind != MethodKind::Constructor && method.name == name
        }
    }
}

fn dispatch_target_kind_name(target: MethodDispatchTarget) -> &'static str {
    match target {
        MethodDispatchTarget::Constructor => method_kind_name(MethodKind::Constructor),
        MethodDispatchTarget::StaticMethod => method_kind_name(MethodKind::Static),
        MethodDispatchTarget::BoundMethod => "method",
    }
}

fn dispatch_target_method_name(target: MethodDispatchTarget, name: &str) -> &str {
    match target {
        MethodDispatchTarget::Constructor => "$init",
        MethodDispatchTarget::StaticMethod | MethodDispatchTarget::BoundMethod => name,
    }
}

fn dispatch_score(
    holder: &raw::Class,
    args: &[JavaOverloadArg],
    expected: &[JavaType],
) -> Result<Option<i32>> {
    if args.len() != expected.len() {
        return Ok(None);
    }

    let mut score = 0;
    for (arg, expected) in args.iter().zip(expected) {
        let Some(arg_score) = dispatch_arg_score(holder, arg, expected)? else {
            return Ok(None);
        };
        score += arg_score;
    }
    Ok(Some(score))
}

fn dispatch_arg_score(
    holder: &raw::Class,
    arg: &JavaOverloadArg,
    expected: &JavaType,
) -> Result<Option<i32>> {
    match arg {
        JavaOverloadArg::RustString(_) => Ok(rust_string_dispatch_score(expected)),
        JavaOverloadArg::Value(JavaValue::Object(None)) => {
            Ok(expected.is_reference().then_some(50))
        }
        JavaOverloadArg::Value(JavaValue::Object(Some(value))) => {
            reference_dispatch_score(holder, value.as_jobject(), expected)
        }
        JavaOverloadArg::Value(value) if primitive_exact_match(*value, expected) => Ok(Some(0)),
        JavaOverloadArg::Value(value) if super::args::can_coerce_java_value(*value, expected) => {
            Ok(Some(10))
        }
        JavaOverloadArg::Value(_) => Ok(None),
    }
}

fn primitive_exact_match(value: JavaValue, expected: &JavaType) -> bool {
    matches!(
        (value, expected),
        (JavaValue::Boolean(_), JavaType::Boolean)
            | (JavaValue::Byte(_), JavaType::Byte)
            | (JavaValue::Char(_), JavaType::Char)
            | (JavaValue::Short(_), JavaType::Short)
            | (JavaValue::Int(_), JavaType::Int)
            | (JavaValue::Long(_), JavaType::Long)
            | (JavaValue::Float(_), JavaType::Float)
            | (JavaValue::Double(_), JavaType::Double)
    )
}

fn rust_string_dispatch_score(expected: &JavaType) -> Option<i32> {
    match expected {
        JavaType::Object(class) if class == "java/lang/String" => Some(0),
        JavaType::Object(class) if class == "java/lang/CharSequence" => Some(1),
        JavaType::Object(class) if class == "java/lang/Object" => Some(2),
        _ => None,
    }
}

fn reference_dispatch_score(
    holder: &raw::Class,
    object: jni::jobject,
    expected: &JavaType,
) -> Result<Option<i32>> {
    if !expected.is_reference() {
        return Ok(None);
    }

    let actual_descriptor = object_class_descriptor(holder, object)?;
    if let Some(score) = reference_descriptor_dispatch_score(&actual_descriptor, expected) {
        return Ok(Some(score));
    }

    let expected_class = class_for_dispatch_type(holder, expected)?;
    let env = holder.vm().attach_current_thread()?;
    if !env.is_instance_of(&RawObject(object), &expected_class.inner.class)? {
        return Ok(None);
    }

    Ok(Some(match expected {
        JavaType::Array(_) => 1,
        JavaType::Object(class) if class == "java/lang/Object" => 30,
        JavaType::Object(_) => 10,
        JavaType::Void
        | JavaType::Boolean
        | JavaType::Byte
        | JavaType::Char
        | JavaType::Short
        | JavaType::Int
        | JavaType::Long
        | JavaType::Float
        | JavaType::Double => unreachable!("non-reference types were rejected above"),
    }))
}

fn reference_descriptor_dispatch_score(
    actual_descriptor: &str,
    expected: &JavaType,
) -> Option<i32> {
    if actual_descriptor == expected.to_string() {
        return Some(0);
    }

    match expected {
        JavaType::Object(class)
            if class == "java/lang/Object"
                && (actual_descriptor.starts_with('L') || actual_descriptor.starts_with('[')) =>
        {
            Some(30)
        }
        _ => None,
    }
}

fn object_class_descriptor(holder: &raw::Class, object: jni::jobject) -> Result<String> {
    let env = holder.vm().attach_current_thread()?;
    let class = env.get_object_class(&RawObject(object))?;
    metadata::class_descriptor(&env, &class)
}

fn class_for_dispatch_type(holder: &raw::Class, ty: &JavaType) -> Result<raw::Class> {
    let env = holder.vm().attach_current_thread()?;
    let java = Java::new(holder.vm().clone());
    let scoped_java = match metadata::class_loader(&env, holder.vm(), holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    scoped_java.find_class(&dispatch_class_lookup_name(ty))
}

fn dispatch_class_lookup_name(ty: &JavaType) -> String {
    match ty {
        JavaType::Object(name) => name.replace('/', "."),
        JavaType::Array(_) => ty.to_string(),
        JavaType::Void
        | JavaType::Boolean
        | JavaType::Byte
        | JavaType::Char
        | JavaType::Short
        | JavaType::Int
        | JavaType::Long
        | JavaType::Float
        | JavaType::Double => ty.to_string(),
    }
}

fn format_dispatch_argument_list(args: &[JavaOverloadArg]) -> String {
    format!(
        "({})",
        args.iter()
            .map(JavaOverloadArg::type_name)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn method_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

fn bind_declared_return(
    holder: &raw::Class,
    ty: &JavaType,
    value: JavaReturn,
    operation: &'static str,
) -> Result<JavaReturn> {
    let JavaType::Object(name) = ty else {
        return Ok(value);
    };
    let JavaReturn::Object(object) = value else {
        return Ok(value);
    };
    let object = match object {
        Some(JavaReturnRef::Object(object)) => object,
        Some(other) => return Ok(JavaReturn::Object(Some(other))),
        None => return Ok(JavaReturn::Object(None)),
    };

    let env = holder.vm().attach_current_thread()?;
    let java = Java::new(holder.vm().clone());
    let scoped_java = match metadata::class_loader(&env, holder.vm(), holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    let class = JavaClass::from_raw(scoped_java.find_class(&name.replace('/', "."))?);
    if class.is_instance(&object)? {
        Ok(JavaReturn::Object(Some(JavaReturnRef::Object(
            object.rebind(class),
        ))))
    } else {
        let actual = env.get_object_class(&object)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "declared return type",
            actual: format!("{:p} is not {}", actual.as_jclass(), name.replace('/', ".")),
        })
    }
}

fn validate_selected_receiver(
    class: &raw::Class,
    object: &(impl JavaObjectRef + ?Sized),
    operation: &'static str,
) -> Result<()> {
    if object.as_jobject().is_null() {
        return Err(Error::NullReturn { operation });
    }

    if class.is_instance(object)? {
        Ok(())
    } else {
        let env = class.vm().attach_current_thread()?;
        let actual = env.get_object_class(object)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "selected member declaring class",
            actual: format!("{:p} is not {}", actual.as_jclass(), class.name()),
        })
    }
}

fn validate_reference_call_args(
    holder: &raw::Class,
    expected: &[JavaType],
    values: &[JavaValue],
) -> Result<()> {
    for (index, (expected, value)) in expected.iter().zip(values).enumerate() {
        if !is_reference_value_assignable(holder, expected, *value)? {
            return Err(Error::InvalidArgumentType {
                index,
                expected: expected.to_string(),
                actual: value.type_name(),
            });
        }
    }
    Ok(())
}

fn validate_reference_field_value(
    holder: &raw::Class,
    expected: &JavaType,
    value: JavaValue,
    operation: &'static str,
) -> Result<()> {
    if is_reference_value_assignable(holder, expected, value)? {
        Ok(())
    } else {
        Err(Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual: value.type_name(),
        })
    }
}

fn is_reference_value_assignable(
    holder: &raw::Class,
    expected: &JavaType,
    value: JavaValue,
) -> Result<bool> {
    let JavaValue::Object(Some(object)) = value else {
        return Ok(true);
    };
    if !expected.is_reference() {
        return Ok(true);
    }

    let expected_class = class_for_dispatch_type(holder, expected)?;
    let env = holder.vm().attach_current_thread()?;
    env.is_instance_of(&RawObject(object.as_jobject()), &expected_class.inner.class)
}

fn field_kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Instance => "instance",
        FieldKind::Static => "static",
    }
}

pub(super) fn select_field_by_name(
    class: &str,
    name: &str,
    fields: Vec<JavaFieldMetadata>,
) -> Result<JavaFieldMetadata> {
    match fields.len() {
        0 => Err(Error::FieldNameNotFound {
            class: class.to_owned(),
            kind: "field",
            name: name.to_owned(),
        }),
        1 => Ok(fields.into_iter().next().expect("one field match")),
        _ => Err(Error::AmbiguousField {
            class: class.to_owned(),
            kind: "field",
            name: name.to_owned(),
            candidates: fields
                .iter()
                .map(|field| format!("{} {}", field_kind_name(field.kind), field.ty))
                .collect(),
        }),
    }
}

fn wrapper_method_name(kind: MethodKind, name: &str) -> &str {
    if kind == MethodKind::Constructor {
        "$init"
    } else {
        name
    }
}

fn select_method_group_by_name(
    class: &str,
    name: &str,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| method.kind != MethodKind::Constructor && method.name == name)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::MethodNameNotFound {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
        }),
        1 => Ok(matches.into_iter().next().expect("one method match")),
        _ => Err(Error::AmbiguousMethod {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            candidates: matches
                .iter()
                .map(|method| format!("{} {}", method_kind_name(method.kind), method.signature))
                .collect(),
        }),
    }
}

#[cfg(test)]
fn select_constructor_by_name(
    class: &str,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| method.kind == MethodKind::Constructor)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::MethodNameNotFound {
            class: class.to_owned(),
            kind: method_kind_name(MethodKind::Constructor),
            name: "$init".to_owned(),
        }),
        1 => Ok(matches.into_iter().next().expect("one constructor match")),
        _ => Err(Error::AmbiguousMethod {
            class: class.to_owned(),
            kind: method_kind_name(MethodKind::Constructor),
            name: "$init".to_owned(),
            candidates: matches
                .iter()
                .map(|method| method.signature.to_string())
                .collect(),
        }),
    }
}

pub(super) fn select_method_by_arguments(
    class: &str,
    kind: MethodKind,
    name: &str,
    arguments: &[JavaType],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| {
            method.kind == kind && method.name == name && method.signature.arguments() == arguments
        })
        .collect::<Vec<_>>();

    select_method_overload_match(class, kind, name, format_argument_list(arguments), matches)
}

fn select_method_group_by_arguments(
    class: &str,
    name: &str,
    arguments: &[JavaType],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| {
            method.kind != MethodKind::Constructor
                && method.name == name
                && method.signature.arguments() == arguments
        })
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::OverloadNotFound {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            arguments: format_argument_list(arguments),
        }),
        1 => Ok(matches.into_iter().next().expect("one overload match")),
        matches => Err(Error::AmbiguousOverload {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            arguments: format_argument_list(arguments),
            matches,
        }),
    }
}

fn select_method_overload_match(
    class: &str,
    kind: MethodKind,
    name: &str,
    arguments: String,
    matches: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    match matches.len() {
        0 => Err(Error::OverloadNotFound {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: wrapper_method_name(kind, name).to_owned(),
            arguments,
        }),
        1 => Ok(matches.into_iter().next().expect("one overload match")),
        matches => Err(Error::AmbiguousOverload {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: wrapper_method_name(kind, name).to_owned(),
            arguments,
            matches,
        }),
    }
}

pub(super) fn parse_type_names(names: &[&str]) -> Result<Vec<JavaType>> {
    names.iter().map(|name| JavaType::from_name(name)).collect()
}

fn format_argument_list(arguments: &[JavaType]) -> String {
    let mut formatted = String::from("(");
    for argument in arguments {
        formatted.push_str(&argument.to_string());
    }
    formatted.push(')');
    formatted
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use crate::{refs::GlobalRef, vm::Vm};

    use super::*;

    const CLASS: &str = "com.example.Subject";

    fn method(name: &str, kind: MethodKind, signature: &str) -> JavaMethodMetadata {
        JavaMethodMetadata {
            name: name.to_owned(),
            kind,
            signature: MethodSignature::parse(signature).unwrap(),
            modifiers: 0,
            id: ptr::null_mut(),
        }
    }

    fn field(name: &str, kind: FieldKind, ty: &str) -> JavaFieldMetadata {
        JavaFieldMetadata {
            name: name.to_owned(),
            kind,
            ty: JavaType::parse(ty).unwrap(),
            modifiers: 0,
            id: ptr::null_mut(),
        }
    }

    fn holder() -> raw::Class {
        let vm = Vm::dangling_for_tests();
        let class = unsafe { GlobalRef::from_raw(vm.clone(), ptr::dangling_mut()) }.unwrap();
        raw::Class::from_global(CLASS.to_owned(), class)
    }

    #[test]
    fn displays_wrapper_metadata_summaries() {
        let class = JavaClass::from_raw(holder());
        assert_eq!(class.class.to_string(), CLASS);
        assert_eq!(class.to_string(), CLASS);
        assert_eq!(class.java_display(), "<class: com.example.Subject>");
        assert!(format!("{class:?}").contains(CLASS));

        let constructor = JavaConstructor {
            class: class.class.clone(),
            metadata: JavaMethodMetadata {
                name: "<init>".to_owned(),
                kind: MethodKind::Constructor,
                signature: MethodSignature::parse("(I)V").unwrap(),
                modifiers: 0,
                id: ptr::dangling_mut(),
            },
        };
        assert_eq!(
            constructor.to_string(),
            "function com.example.Subject.<init>(I)V"
        );
        assert!(format!("{constructor:?}").contains("JavaConstructor"));
        assert_eq!(
            constructor.java_display(),
            "function com.example.Subject.<init>(I)V"
        );

        let method = JavaMethod {
            class: class.class.clone(),
            metadata: JavaMethodMetadata {
                name: "answer".to_owned(),
                kind: MethodKind::Static,
                signature: MethodSignature::parse("()I").unwrap(),
                modifiers: 0,
                id: ptr::dangling_mut(),
            },
        };
        assert_eq!(method.to_string(), "function com.example.Subject.answer()I");
        assert!(format!("{method:?}").contains("JavaMethod"));
        assert_eq!(
            method.java_display(),
            "function com.example.Subject.answer()I"
        );

        let field = JavaField {
            class: class.class.clone(),
            metadata: JavaFieldMetadata {
                name: "number".to_owned(),
                kind: FieldKind::Instance,
                ty: JavaType::Int,
                modifiers: 0,
                id: ptr::dangling_mut(),
            },
        };
        assert_eq!(field.to_string(), "field com.example.Subject.number: I");
        assert!(format!("{field:?}").contains("JavaField"));
        assert_eq!(field.java_display(), "field com.example.Subject.number: I");

        let object =
            unsafe { JavaObject::from_global_raw(class.clone(), ptr::dangling_mut()) }.unwrap();

        let bound_method = JavaBoundMethodOverload {
            object: &object,
            overload: method,
        };
        assert!(format!("{bound_method:?}").contains("JavaBoundMethodOverload"));

        let bound_field = JavaBoundFieldHandle {
            object: &object,
            field,
        };
        assert!(format!("{bound_field:?}").contains("JavaBoundFieldHandle"));
    }

    #[test]
    fn resolves_string_selector_for_unambiguous_method() {
        let selected = select_method_group_by_name(
            CLASS,
            "onResume",
            vec![method("onResume", MethodKind::Instance, "()V")],
        )
        .unwrap();

        assert_eq!(selected.name, "onResume");
        assert_eq!(selected.signature.to_string(), "()V");
    }

    #[test]
    fn resolves_unambiguous_instance_field_selector() {
        let selected = select_field_by_name(
            CLASS,
            "number",
            vec![field("number", FieldKind::Instance, "I")],
        )
        .unwrap();

        assert_eq!(selected.name, "number");
        assert_eq!(selected.kind, FieldKind::Instance);
        assert_eq!(selected.ty, JavaType::Int);
    }

    #[test]
    fn resolves_unambiguous_static_field_selector() {
        let selected = select_field_by_name(
            CLASS,
            "answer",
            vec![field("answer", FieldKind::Static, "Ljava/lang/String;")],
        )
        .unwrap();

        assert_eq!(selected.name, "answer");
        assert_eq!(selected.kind, FieldKind::Static);
        assert_eq!(selected.ty, JavaType::Object("java/lang/String".to_owned()));
    }

    #[test]
    fn reports_missing_field_selector() {
        let error = select_field_by_name(CLASS, "missing", vec![]).unwrap_err();

        match error {
            Error::FieldNameNotFound {
                class,
                kind: "field",
                name,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "missing");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_same_tier_field_selector_with_candidate_kinds() {
        let selected = select_field_by_name(
            CLASS,
            "sameName",
            vec![
                field("sameName", FieldKind::Instance, "I"),
                field("sameName", FieldKind::Static, "J"),
            ],
        )
        .unwrap_err();

        match selected {
            Error::AmbiguousField {
                class,
                kind: "field",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "sameName");
                assert_eq!(
                    candidates,
                    vec!["instance I".to_owned(), "static J".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolves_type_list_selector_for_overload() {
        let arguments = parse_type_names(&["java.lang.String", "int"]).unwrap();
        let selected = select_method_by_arguments(
            CLASS,
            MethodKind::Instance,
            "set",
            &arguments,
            vec![
                method("set", MethodKind::Instance, "(I)V"),
                method("set", MethodKind::Instance, "(Ljava/lang/String;I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(Ljava/lang/String;I)V");
    }

    #[test]
    fn reports_missing_type_list_overload() {
        let arguments = parse_type_names(&["java.lang.String"]).unwrap();
        let error = select_method_by_arguments(
            CLASS,
            MethodKind::Instance,
            "set",
            &arguments,
            vec![method("set", MethodKind::Instance, "(I)V")],
        )
        .unwrap_err();

        match error {
            Error::OverloadNotFound {
                class,
                kind: "instance",
                name,
                arguments,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "set");
                assert_eq!(arguments, "(Ljava/lang/String;)");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_bare_name_with_candidate_signatures() {
        let error = select_method_group_by_name(
            CLASS,
            "overload",
            vec![
                method("overload", MethodKind::Instance, "()I"),
                method("overload", MethodKind::Instance, "(I)I"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousMethod {
                class,
                kind: "method",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "overload");
                assert_eq!(
                    candidates,
                    vec!["instance ()I".to_owned(), "instance (I)I".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolves_unambiguous_constructor_for_class_new() {
        let selected = select_constructor_by_name(
            CLASS,
            vec![
                method("ignored", MethodKind::Static, "()I"),
                method("<init>", MethodKind::Constructor, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.name, "<init>");
        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn reports_missing_constructor_for_class_new() {
        let error =
            select_constructor_by_name(CLASS, vec![method("answer", MethodKind::Static, "()I")])
                .unwrap_err();

        match error {
            Error::MethodNameNotFound {
                class,
                kind: "constructor",
                name,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "$init");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_constructor_for_class_new() {
        let error = select_constructor_by_name(
            CLASS,
            vec![
                method("<init>", MethodKind::Constructor, "()V"),
                method("<init>", MethodKind::Constructor, "(I)V"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousMethod {
                class,
                kind: "constructor",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "$init");
                assert_eq!(candidates, vec!["()V".to_owned(), "(I)V".to_owned()]);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn replacement_selector_accepts_static_or_instance_method() {
        let selected = select_method_group_by_name(
            CLASS,
            "answer",
            vec![method("answer", MethodKind::Static, "()I")],
        )
        .unwrap();
        assert_eq!(selected.kind, MethodKind::Static);
        assert_eq!(selected.signature.to_string(), "()I");

        let arguments = parse_type_names(&["java.lang.String"]).unwrap();
        let selected = select_method_group_by_arguments(
            CLASS,
            "message",
            &arguments,
            vec![method(
                "message",
                MethodKind::Instance,
                "(Ljava/lang/String;)Ljava/lang/String;",
            )],
        )
        .unwrap();
        assert_eq!(selected.kind, MethodKind::Instance);
        assert_eq!(
            selected.signature.to_string(),
            "(Ljava/lang/String;)Ljava/lang/String;"
        );
    }

    #[test]
    fn replacement_selector_reports_static_instance_ambiguity() {
        let error = select_method_group_by_arguments(
            CLASS,
            "sameShape",
            &[],
            vec![
                method("sameShape", MethodKind::Instance, "()I"),
                method("sameShape", MethodKind::Static, "()I"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousOverload {
                class,
                kind: "method",
                name,
                arguments,
                matches: 2,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "sameShape");
                assert_eq!(arguments, "()");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dispatch_filters_by_arity() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "set",
            &[JavaOverloadArg::Value(JavaValue::Int(7))],
            vec![
                method("set", MethodKind::Instance, "()V"),
                method("set", MethodKind::Instance, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn bound_dispatch_reports_method_failures() {
        let error = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "set",
            &[JavaOverloadArg::Value(JavaValue::Int(7))],
            vec![
                method("set", MethodKind::Instance, "()V"),
                method("set", MethodKind::Static, "()V"),
            ],
        )
        .unwrap_err();

        match error {
            Error::NoCompatibleOverload {
                class,
                kind: "method",
                name,
                arguments,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "set");
                assert_eq!(arguments, "(int)");
                assert_eq!(
                    candidates,
                    vec!["instance ()V".to_owned(), "static ()V".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dispatch_prefers_exact_primitive_over_coercion() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::StaticMethod,
            "number",
            &[JavaOverloadArg::Value(JavaValue::Int(7))],
            vec![
                method("number", MethodKind::Static, "(J)V"),
                method("number", MethodKind::Static, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn dispatch_ranks_rust_string_targets() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "text",
            &[JavaOverloadArg::RustString("hello".to_owned())],
            vec![
                method("text", MethodKind::Instance, "(Ljava/lang/Object;)V"),
                method("text", MethodKind::Instance, "(Ljava/lang/CharSequence;)V"),
                method("text", MethodKind::Instance, "(Ljava/lang/String;)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(Ljava/lang/String;)V");
    }

    #[test]
    fn dispatch_preserves_order_for_tied_scores() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "nullable",
            &[JavaOverloadArg::Value(JavaValue::NULL)],
            vec![
                method(
                    "nullable",
                    MethodKind::Instance,
                    "(Ljava/lang/CharSequence;)V",
                ),
                method("nullable", MethodKind::Instance, "(Ljava/lang/String;)V"),
            ],
        )
        .unwrap();

        assert_eq!(
            selected.signature.to_string(),
            "(Ljava/lang/CharSequence;)V"
        );
    }

    #[test]
    fn array_descriptor_exact_match_scores_before_object() {
        assert_eq!(
            reference_descriptor_dispatch_score("[I", &JavaType::Array(Box::new(JavaType::Int))),
            Some(0)
        );
        assert_eq!(
            reference_descriptor_dispatch_score(
                "[I",
                &JavaType::Object("java/lang/Object".to_owned())
            ),
            Some(30)
        );
    }
}
