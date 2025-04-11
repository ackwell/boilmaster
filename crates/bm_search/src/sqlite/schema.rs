use bm_read::LanguageString;
use ironworks::{excel::Language, file::exh};
use sea_query::{Alias, ColumnType, Iden};

pub fn table_name(sheet_name: &str, language: Language) -> Alias {
	let language_string = LanguageString::from(language);
	Alias::new(format!("sheet-{sheet_name}@{language_string}"))
}

#[derive(Iden)]
pub enum KnownColumn {
	RowId,
	SubrowId,
}

pub fn column_name(column: &exh::ColumnDefinition) -> Alias {
	let offset = column.offset;

	// For packed bool columns, offset alone is not enough to disambiguate a
	// field - add a suffix of the packed bit position.
	use exh::ColumnKind as CK;
	let suffix = match column.kind {
		CK::PackedBool0 => "_0",
		CK::PackedBool1 => "_1",
		CK::PackedBool2 => "_2",
		CK::PackedBool3 => "_3",
		CK::PackedBool4 => "_4",
		CK::PackedBool5 => "_5",
		CK::PackedBool6 => "_6",
		CK::PackedBool7 => "_7",
		_ => "",
	};

	Alias::new(format!("{offset}{suffix}"))
}

pub fn column_type(column: &exh::ColumnDefinition) -> ColumnType {
	use exh::ColumnKind as CK;
	match column.kind {
		// Using text for this because we have absolutely no idea how large any given string is going to be.
		CK::String => ColumnType::Text,

		// Pretty much all of this will collapse to "INTEGER" on sqlite but hey. Accuracy.
		CK::Int8 => ColumnType::TinyInteger,
		CK::UInt8 => ColumnType::TinyUnsigned,
		CK::Int16 => ColumnType::SmallInteger,
		CK::UInt16 => ColumnType::SmallUnsigned,
		CK::Int32 => ColumnType::Integer,
		CK::UInt32 => ColumnType::Unsigned,
		CK::Int64 => ColumnType::BigInteger,
		CK::UInt64 => ColumnType::BigUnsigned,
		CK::Float32 => ColumnType::Float,

		CK::Bool
		| CK::PackedBool0
		| CK::PackedBool1
		| CK::PackedBool2
		| CK::PackedBool3
		| CK::PackedBool4
		| CK::PackedBool5
		| CK::PackedBool6
		| CK::PackedBool7 => ColumnType::Boolean,
	}
}
