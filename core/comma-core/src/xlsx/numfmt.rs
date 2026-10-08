//! Excel number formats: how a stored number is shown. `#,##0.00`, `0%`, `0.00E+00`,
//! `# ?/?`, `dd.mm.yyyy h:mm`, `[h]:mm`, sections for negative numbers, zero and text,
//! conditions such as `[>=1000]`. Format codes in a file are always written the American way;
//! separators, the short date and the names of months and days come from the locale, as in
//! Excel.

use crate::formula::civil_from_days;

/// What Excel takes from the system to show numbers and dates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayLocale {
    pub decimal_separator: char,
    pub group_separator: char,
    /// The format of the built-in short date (format 14), such as `dd.mm.yyyy`.
    pub short_date: String,
    /// January first.
    pub month_names: Vec<String>,
    pub month_abbreviations: Vec<String>,
    /// Sunday first.
    pub day_names: Vec<String>,
    pub day_abbreviations: Vec<String>,
    pub am: String,
    pub pm: String,
    pub true_text: String,
    pub false_text: String,
}

impl Default for DisplayLocale {
    /// English (United States).
    fn default() -> Self {
        let names = |list: &str| list.split(' ').map(str::to_owned).collect();
        Self {
            decimal_separator: '.',
            group_separator: ',',
            short_date: "m/d/yyyy".to_owned(),
            month_names: names("January February March April May June July August September October November December"),
            month_abbreviations: names("Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec"),
            day_names: names("Sunday Monday Tuesday Wednesday Thursday Friday Saturday"),
            day_abbreviations: names("Sun Mon Tue Wed Thu Fri Sat"),
            am: "AM".to_owned(),
            pm: "PM".to_owned(),
            true_text: "TRUE".to_owned(),
            false_text: "FALSE".to_owned(),
        }
    }
}

/// The code of a built-in format, `None` for General and the ids nothing is known about.
pub(crate) fn builtin_code(id: u32, locale: &DisplayLocale) -> Option<String> {
    let code = match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "#,##0 ;(#,##0)",
        6 => "#,##0 ;[Red](#,##0)",
        7 => "#,##0.00;(#,##0.00)",
        8 => "#,##0.00;[Red](#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 | 27..=31 | 34..=36 | 50..=58 => return Some(locale.short_date.clone()),
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 | 32 => "h:mm",
        21 | 33 => "h:mm:ss",
        22 => return Some(format!("{} h:mm", locale.short_date)),
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* (#,##0);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* (#,##0.00);_(\"$\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    };
    Some(code.to_owned())
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Literal(String),
    /// Digit placeholders `0`, `#` and `?`.
    Zero,
    Hash,
    Question,
    Point,
    Comma,
    Percent,
    /// `E+` (`true`) or `E-`.
    Exponent(bool),
    Slash,
    At,
    General,
    Year(usize),
    /// `m`: a month, or minutes next to hours or seconds; told apart after reading.
    Month(usize),
    Minute(usize),
    Day(usize),
    Hour(usize),
    Second(usize),
    /// Digits of a second after `ss.`.
    SubSecond(usize),
    /// `AM/PM` (`true`) or `A/P`, with the case of the code.
    AmPm(bool, bool),
    Elapsed(Unit, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Hours,
    Minutes,
    Seconds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

impl Comparison {
    fn test(self, value: f64, limit: f64) -> bool {
        match self {
            Self::Less => value < limit,
            Self::LessOrEqual => value <= limit,
            Self::Greater => value > limit,
            Self::GreaterOrEqual => value >= limit,
            Self::Equal => value == limit,
            Self::NotEqual => value != limit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Number,
    Fraction,
    Date,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
struct Section {
    tokens: Vec<Token>,
    condition: Option<(Comparison, f64)>,
    kind: Kind,
}

/// A compiled format code.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NumberFormat {
    /// Up to three sections for numbers: positive (or all), negative, zero.
    numbers: Vec<Section>,
    text: Option<Section>,
}

impl NumberFormat {
    pub(crate) fn general() -> Self {
        Self::parse("General")
    }

    pub(crate) fn parse(code: &str) -> Self {
        let mut numbers = Vec::new();
        let mut text = None;
        for part in split_sections(code) {
            let section = parse_section(part);
            if section.kind == Kind::Text && text.is_none() {
                text = Some(section);
            } else if numbers.len() < 3 {
                numbers.push(section);
            }
        }
        if numbers.is_empty() {
            numbers.push(parse_section("General"));
        }
        Self { numbers, text }
    }

    /// Text in the text section, if there is one. Padding around it goes; the text itself
    /// stays as it is.
    /// Whether numbers show as dates or times.
    pub(crate) fn is_date(&self) -> bool {
        self.numbers[0].kind == Kind::Date
    }

    pub(crate) fn format_text(&self, text: &str) -> String {
        let Some(section) = &self.text else {
            return text.to_owned();
        };
        let tokens = &section.tokens;
        let first = tokens.iter().position(|token| *token == Token::At).unwrap_or(tokens.len());
        let last = tokens.iter().rposition(|token| *token == Token::At).map_or(tokens.len(), |last| last + 1);
        let render = |tokens: &[Token]| {
            let mut out = String::new();
            for token in tokens {
                match token {
                    Token::At => out.push_str(text),
                    token => push_plain(&mut out, token),
                }
            }
            out
        };
        let head = render(&tokens[..first]);
        let tail = render(&tokens[last.max(first)..]);
        format!(
            "{}{}{}",
            head.trim_start_matches(' '),
            render(&tokens[first..last.max(first)]),
            tail.trim_end_matches(' ')
        )
    }

    pub(crate) fn format_number(&self, value: f64, locale: &DisplayLocale, date1904: bool) -> String {
        if !value.is_finite() {
            return "#NUM!".to_owned();
        }
        let (section, value, sign) = self.section_for(value);
        let text = match section.kind {
            Kind::Date => match DateTime::from_serial(value, date1904, subsecond_digits(section)) {
                Some(date) => format_date(section, &date, value, locale),
                None => general(value, locale),
            },
            Kind::Fraction => format_fraction(section, value.abs(), locale),
            Kind::Number | Kind::Text => format_digits(section, value.abs(), locale),
        };
        let text = trim_padding(text);
        // A value that rounds to zero is shown without a minus.
        if sign && text.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
            format!("-{text}")
        } else {
            text
        }
    }

    /// The section a number is shown with, the value it shows (dates keep the sign, as a
    /// negative date is not shown as one) and whether a minus goes in front.
    fn section_for(&self, value: f64) -> (&Section, f64, bool) {
        let numbers = &self.numbers;
        if numbers.iter().any(|section| section.condition.is_some()) {
            for section in numbers {
                if let Some((comparison, limit)) = section.condition
                    && comparison.test(value, limit)
                {
                    // `[<0]` and the like show the number without its minus.
                    let implied_negative = matches!(comparison, Comparison::Less | Comparison::LessOrEqual) && limit <= 0.0;
                    return (section, value, value < 0.0 && !implied_negative);
                }
            }
            let fallback = numbers.iter().find(|section| section.condition.is_none()).unwrap_or(&numbers[0]);
            return (fallback, value, value < 0.0);
        }
        match numbers.len() {
            1 => (&numbers[0], value, value < 0.0 && numbers[0].kind != Kind::Date),
            _ if value < 0.0 => (&numbers[1], value.abs(), false),
            3 if value == 0.0 => (&numbers[2], value, false),
            _ => (&numbers[0], value, false),
        }
    }
}

// MARK: Reading a code

/// Sections split at `;` outside quotes, brackets and escapes.
fn split_sections(code: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (index, c) in code.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !quoted => escaped = true,
            '"' if !bracket => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                sections.push(&code[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    sections.push(&code[start..]);
    sections
}

fn parse_section(code: &str) -> Section {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut condition = None;
    let mut i = 0;
    let starts_with = |i: usize, word: &str| {
        let word: Vec<char> = word.chars().collect();
        chars.len() >= i + word.len() && chars[i..i + word.len()].iter().zip(&word).all(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    while i < chars.len() {
        let c = chars[i];
        let run = chars[i..].iter().take_while(|&&next| next.eq_ignore_ascii_case(&c)).count();
        match c {
            '"' => {
                let end = chars[i + 1..].iter().position(|&next| next == '"').map_or(chars.len(), |p| i + 1 + p);
                tokens.push(Token::Literal(chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    tokens.push(Token::Literal(next.to_string()));
                }
                i += 2;
                continue;
            }
            // Padding as wide as the next character, and a fill to the cell's width: in a
            // table without fixed widths both are a space at most.
            '_' => {
                tokens.push(Token::Literal(" ".to_owned()));
                i += 2;
                continue;
            }
            '*' => {
                i += 2;
                continue;
            }
            '[' => {
                let end = chars[i + 1..].iter().position(|&next| next == ']').map_or(chars.len(), |p| i + 1 + p);
                let inside: String = chars[i + 1..end].iter().collect();
                if let Some(token) = bracket_token(&inside, &mut condition) {
                    tokens.push(token);
                }
                i = end + 1;
                continue;
            }
            '0' => tokens.push(Token::Zero),
            '#' => tokens.push(Token::Hash),
            '?' => tokens.push(Token::Question),
            '.' => tokens.push(Token::Point),
            ',' => tokens.push(Token::Comma),
            '%' => tokens.push(Token::Percent),
            '@' => tokens.push(Token::At),
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                tokens.push(Token::Exponent(chars[i + 1] == '+'));
                i += 2;
                continue;
            }
            _ if starts_with(i, "General") => {
                tokens.push(Token::General);
                i += "General".len();
                continue;
            }
            _ if starts_with(i, "AM/PM") => {
                tokens.push(Token::AmPm(true, c.is_ascii_uppercase()));
                i += 5;
                continue;
            }
            _ if starts_with(i, "A/P") => {
                tokens.push(Token::AmPm(false, c.is_ascii_uppercase()));
                i += 3;
                continue;
            }
            '/' => tokens.push(Token::Slash),
            'y' | 'Y' | 'e' => {
                tokens.push(Token::Year(if c == 'e' { 4 } else { run }));
                i += run;
                continue;
            }
            'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                tokens.push(match c.to_ascii_lowercase() {
                    'm' => Token::Month(run),
                    'd' => Token::Day(run),
                    'h' => Token::Hour(run),
                    _ => Token::Second(run),
                });
                i += run;
                continue;
            }
            // Calendar and era letters: nothing to show.
            'g' | 'G' | 'b' | 'B' => {}
            _ => tokens.push(Token::Literal(c.to_string())),
        }
        i += 1;
    }
    let kind = section_kind(&tokens);
    if kind == Kind::Date {
        resolve_date_tokens(&mut tokens);
    }
    Section { tokens, condition, kind }
}

/// Elapsed time, a currency or a condition in brackets; colours and locales show nothing.
fn bracket_token(inside: &str, condition: &mut Option<(Comparison, f64)>) -> Option<Token> {
    let lower = inside.to_ascii_lowercase();
    if !lower.is_empty() && lower.bytes().all(|b| b == lower.as_bytes()[0]) {
        let unit = match lower.as_bytes()[0] {
            b'h' => Some(Unit::Hours),
            b'm' => Some(Unit::Minutes),
            b's' => Some(Unit::Seconds),
            _ => None,
        };
        if let Some(unit) = unit {
            return Some(Token::Elapsed(unit, lower.len()));
        }
    }
    if let Some(currency) = inside.strip_prefix('$') {
        let symbol = currency.split('-').next().unwrap_or_default();
        return (!symbol.is_empty()).then(|| Token::Literal(symbol.to_owned()));
    }
    let (comparison, rest) = [
        ("<=", Comparison::LessOrEqual),
        (">=", Comparison::GreaterOrEqual),
        ("<>", Comparison::NotEqual),
        ("<", Comparison::Less),
        (">", Comparison::Greater),
        ("=", Comparison::Equal),
    ]
    .into_iter()
    .find_map(|(prefix, comparison)| inside.strip_prefix(prefix).map(|rest| (comparison, rest)))?;
    if let Ok(limit) = rest.trim().parse::<f64>() {
        *condition = Some((comparison, limit));
    }
    None
}

fn section_kind(tokens: &[Token]) -> Kind {
    let is_date = tokens.iter().any(|token| {
        matches!(
            token,
            Token::Year(_)
                | Token::Month(_)
                | Token::Day(_)
                | Token::Hour(_)
                | Token::Second(_)
                | Token::AmPm(..)
                | Token::Elapsed(..)
        )
    });
    let is_digit = |token: &Token| matches!(token, Token::Zero | Token::Hash | Token::Question);
    if is_date {
        Kind::Date
    } else if tokens.contains(&Token::At) && !tokens.iter().any(is_digit) {
        Kind::Text
    } else if let Some(slash) = tokens.iter().position(|token| *token == Token::Slash)
        && tokens[..slash].iter().any(is_digit)
        && tokens[slash + 1..].iter().any(|token| is_digit(token) || is_literal_digit(token))
    {
        Kind::Fraction
    } else {
        Kind::Number
    }
}

fn is_literal_digit(token: &Token) -> bool {
    matches!(token, Token::Literal(text) if text.len() == 1 && text.as_bytes()[0].is_ascii_digit())
}

/// `m` next to hours or before seconds means minutes; `.0` after seconds is a part of a second.
fn resolve_date_tokens(tokens: &mut Vec<Token>) {
    let is_time_part = |token: &Token| {
        matches!(token, Token::Year(_) | Token::Month(_) | Token::Day(_) | Token::Hour(_) | Token::Second(_) | Token::Elapsed(..))
    };
    for index in 0..tokens.len() {
        let Token::Month(count) = tokens[index] else { continue };
        if count > 2 {
            continue;
        }
        let previous = tokens[..index].iter().rev().find(|token| is_time_part(token));
        let next = tokens[index + 1..].iter().find(|token| is_time_part(token));
        let after_hours = matches!(previous, Some(Token::Hour(_) | Token::Elapsed(Unit::Hours, _)));
        let before_seconds = matches!(next, Some(Token::Second(_) | Token::Elapsed(Unit::Seconds, _)));
        if after_hours || before_seconds {
            tokens[index] = Token::Minute(count);
        }
    }
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == Token::Point
            && index > 0
            && matches!(tokens[index - 1], Token::Second(_) | Token::Elapsed(Unit::Seconds, _))
        {
            let digits = tokens[index + 1..].iter().take_while(|token| **token == Token::Zero).count();
            if digits > 0 {
                tokens.splice(index..index + 1 + digits, [Token::SubSecond(digits)]);
            }
        }
        index += 1;
    }
}

/// Spaces from `_` padding and `?` placeholders mean alignment in a column of fixed width;
/// in a table they only stand in the way of reading and comparing values. A shown number
/// has no spaces of its own (the group separator is another character), so runs of them go.
fn trim_padding(text: String) -> String {
    if !text.contains(' ') {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    for c in text.trim_matches(' ').chars() {
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
    }
    out
}

fn push_plain(out: &mut String, token: &Token) {
    match token {
        Token::Literal(text) => out.push_str(text),
        Token::Zero => out.push('0'),
        Token::Point => out.push('.'),
        Token::Comma => out.push(','),
        Token::Percent => out.push('%'),
        Token::Slash => out.push('/'),
        _ => {}
    }
}

// MARK: Numbers

/// A positive number as decimal digits: `digits` with the point after `point` of them, rounded
/// to the 15 significant digits Excel keeps.
#[derive(Clone, Debug)]
struct Decimal {
    digits: Vec<u8>,
    point: i32,
}

impl Decimal {
    fn new(value: f64) -> Self {
        let value = value.abs();
        if value == 0.0 {
            return Self { digits: Vec::new(), point: 0 };
        }
        let text = format!("{value:.14e}");
        let (mantissa, exponent) = text.split_once('e').expect("exponent form");
        let exponent: i32 = exponent.parse().expect("exponent");
        let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
        while digits.last() == Some(&b'0') {
            digits.pop();
        }
        Self { digits, point: exponent + 1 }
    }

    /// Integer and fraction digits with `places` digits after the point, rounded half away
    /// from zero. The integer part is empty below one.
    fn fixed(&self, places: usize) -> (String, String) {
        let mut digits = self.digits.clone();
        let mut point = self.point;
        let keep = point + places as i32;
        if keep < 0 {
            digits.clear();
        } else if (keep as usize) < digits.len() {
            let round_up = digits[keep as usize] >= b'5';
            digits.truncate(keep as usize);
            if round_up {
                let mut index = digits.len();
                loop {
                    if index == 0 {
                        digits.insert(0, b'1');
                        point += 1;
                        break;
                    }
                    index -= 1;
                    if digits[index] == b'9' {
                        digits[index] = b'0';
                    } else {
                        digits[index] += 1;
                        break;
                    }
                }
            }
        }
        let digit = |position: i32| {
            let digit = if position < 0 { b'0' } else { digits.get(position as usize).copied().unwrap_or(b'0') };
            digit as char
        };
        let integer: String = (0..point.max(0)).map(digit).collect();
        let fraction: String = (point..point + places as i32).map(digit).collect();
        (integer.trim_start_matches('0').to_owned(), fraction)
    }
}

/// Excel's General: integers in full, other numbers to 15 significant digits, very large and
/// very small ones in scientific notation.
pub(crate) fn general(value: f64, locale: &DisplayLocale) -> String {
    // Most numbers need no rounding: integers, and fractions whose shortest form has at most
    // 15 significant digits.
    let magnitude = value.abs();
    if value.fract() == 0.0 && magnitude < 1e15 {
        return (value as i64).to_string();
    }
    if (1e-4..1e15).contains(&magnitude) {
        let shortest = value.to_string();
        let significant = shortest.trim_start_matches(['-', '0', '.']).bytes().filter(u8::is_ascii_digit).count();
        if significant <= 15 {
            return if locale.decimal_separator == '.' { shortest } else { shortest.replace('.', &locale.decimal_separator.to_string()) };
        }
    }
    let decimal = Decimal::new(value);
    let sign = if value < 0.0 { "-" } else { "" };
    if decimal.digits.is_empty() {
        return "0".to_owned();
    }
    let point = decimal.point;
    let digits = String::from_utf8(decimal.digits).expect("ASCII");
    if !(-8..=15).contains(&point) {
        let (first, rest) = digits.split_at(1);
        let exponent = point - 1;
        let mantissa = if rest.is_empty() { first.to_owned() } else { format!("{first}{}{rest}", locale.decimal_separator) };
        return format!("{sign}{mantissa}E{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.abs());
    }
    if point <= 0 {
        return format!("{sign}0{}{}{digits}", locale.decimal_separator, "0".repeat(point.unsigned_abs() as usize));
    }
    let point = point as usize;
    if digits.len() <= point {
        format!("{sign}{digits}{}", "0".repeat(point - digits.len()))
    } else {
        format!("{sign}{}{}{}", &digits[..point], locale.decimal_separator, &digits[point..])
    }
}

fn is_placeholder(token: &Token) -> bool {
    matches!(token, Token::Zero | Token::Hash | Token::Question)
}

fn missing_digit(token: &Token) -> &'static str {
    match token {
        Token::Zero => "0",
        Token::Question => " ",
        _ => "",
    }
}

/// Fills placeholders from the right with `digits`; the leftmost takes all that remain.
fn fill_from_right(placeholders: &[&Token], digits: &str) -> Vec<String> {
    let digits: Vec<char> = digits.chars().collect();
    let mut remaining = digits.len();
    let mut filled = vec![String::new(); placeholders.len()];
    for (index, placeholder) in placeholders.iter().enumerate().rev() {
        filled[index] = if index == 0 && remaining > 0 {
            digits[..remaining].iter().collect()
        } else if remaining > 0 {
            remaining -= 1;
            digits[remaining].to_string()
        } else {
            missing_digit(placeholder).to_owned()
        };
    }
    filled
}

fn format_digits(section: &Section, value: f64, locale: &DisplayLocale) -> String {
    let tokens = &section.tokens;
    if tokens.contains(&Token::General) {
        let mut out = String::new();
        for token in tokens {
            match token {
                Token::General => out.push_str(&general(value, locale)),
                token => push_plain(&mut out, token),
            }
        }
        return out;
    }
    let point = tokens.iter().position(|token| *token == Token::Point);
    let exponent = tokens.iter().position(|token| matches!(token, Token::Exponent(_)));
    let integer_end = point.or(exponent).unwrap_or(tokens.len());
    let fraction_end = exponent.unwrap_or(tokens.len());

    // Commas between integer placeholders group thousands; commas right after the last
    // placeholder of the number divide it by a thousand each.
    let first_integer_placeholder = tokens[..integer_end].iter().position(is_placeholder);
    let last_integer_placeholder = tokens[..integer_end].iter().rposition(is_placeholder);
    let last_placeholder = tokens[..fraction_end].iter().rposition(is_placeholder);
    let mut grouping = false;
    let mut scale = 0;
    let mut silent_commas = vec![false; tokens.len()];
    for (index, token) in tokens[..fraction_end].iter().enumerate() {
        if *token != Token::Comma {
            continue;
        }
        if let (Some(first), Some(last)) = (first_integer_placeholder, last_integer_placeholder)
            && index > first
            && index < last
        {
            grouping = true;
            silent_commas[index] = true;
        } else if let Some(last) = last_placeholder
            && index > last
            && tokens[last + 1..index].iter().all(|t| *t == Token::Comma)
        {
            scale += 1;
            silent_commas[index] = true;
        }
    }
    let percents = tokens.iter().filter(|token| **token == Token::Percent).count();
    let value = value * 100f64.powi(percents as i32) / 1000f64.powi(scale);

    let integer_placeholders: Vec<&Token> = tokens[..integer_end].iter().filter(|t| is_placeholder(t)).collect();
    let fraction_placeholders: Vec<&Token> = match point {
        Some(point) => tokens[point + 1..fraction_end].iter().filter(|t| is_placeholder(t)).collect(),
        None => Vec::new(),
    };
    let places = fraction_placeholders.len();

    let mut decimal = Decimal::new(value);
    let mut exponent_value = 0i32;
    if exponent.is_some() && !decimal.digits.is_empty() {
        let width = integer_placeholders.len().max(1) as i32;
        let magnitude = decimal.point - 1;
        exponent_value = if integer_placeholders.contains(&&Token::Hash) {
            magnitude.div_euclid(width) * width
        } else {
            magnitude - (width - 1)
        };
        decimal.point -= exponent_value;
        // Rounding can carry into a new digit: 9.99 → 10.0.
        let (integer, _) = decimal.fixed(places);
        if integer.len() as i32 > width && !integer_placeholders.contains(&&Token::Hash) {
            exponent_value += 1;
            decimal.point -= 1;
        }
    }
    let (integer, fraction) = decimal.fixed(places);

    // Integer placeholders, each with what it shows.
    let integer_parts: Vec<String> = if grouping {
        let zeros = integer_placeholders.iter().position(|t| **t == Token::Zero).map_or(0, |p| integer_placeholders.len() - p);
        let padded = format!("{}{integer}", "0".repeat(zeros.saturating_sub(integer.len())));
        let mut parts = vec![String::new(); integer_placeholders.len()];
        if let Some(first) = parts.first_mut() {
            *first = group(&padded, locale.group_separator);
        }
        parts
    } else {
        fill_from_right(&integer_placeholders, &integer)
    };
    // Fraction placeholders: trailing zeros vanish under `#` and become spaces under `?`.
    let significant = fraction.trim_end_matches('0').len();
    let fraction_parts: Vec<String> = fraction_placeholders
        .iter()
        .zip(fraction.chars())
        .enumerate()
        .map(|(index, (placeholder, digit))| {
            if index < significant { digit.to_string() } else { missing_digit(placeholder).to_owned() }
        })
        .collect();
    let exponent_parts: Vec<String> = match exponent {
        Some(position) => {
            let placeholders: Vec<&Token> = tokens[position + 1..].iter().filter(|t| is_placeholder(t)).collect();
            fill_from_right(&placeholders, &exponent_value.unsigned_abs().to_string())
        }
        None => Vec::new(),
    };

    let mut out = String::new();
    let (mut integer_index, mut fraction_index, mut exponent_index) = (0, 0, 0);
    // A number with no integer placeholder still shows its integer digits, before the point.
    let mut pending_integer = if integer_placeholders.is_empty() && !integer.is_empty() { Some(integer.clone()) } else { None };
    for (index, token) in tokens.iter().enumerate() {
        let in_fraction = point.is_some_and(|point| index > point) && index < fraction_end;
        let in_exponent = exponent.is_some_and(|exponent| index > exponent);
        match token {
            Token::Zero | Token::Hash | Token::Question => {
                if in_exponent {
                    out.push_str(&exponent_parts[exponent_index]);
                    exponent_index += 1;
                } else if in_fraction {
                    out.push_str(&fraction_parts[fraction_index]);
                    fraction_index += 1;
                } else {
                    out.push_str(&integer_parts[integer_index]);
                    integer_index += 1;
                }
            }
            Token::Point if Some(index) == point => {
                if let Some(integer) = pending_integer.take() {
                    out.push_str(&integer);
                }
                out.push(locale.decimal_separator);
            }
            Token::Exponent(plus) => {
                if let Some(integer) = pending_integer.take() {
                    out.push_str(&integer);
                }
                out.push('E');
                if exponent_value < 0 {
                    out.push('-');
                } else if *plus {
                    out.push('+');
                }
            }
            Token::Comma if silent_commas[index] => {}
            token => push_plain(&mut out, token),
        }
    }
    if let Some(integer) = pending_integer {
        out.push_str(&integer);
    }
    out
}

fn group(digits: &str, separator: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * separator.len_utf8());
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

/// `# ?/?`, `?/8`, `0 ??/100`: a whole part, if the code has one, and a fraction with the
/// closest denominator of at most as many digits as placeholders, or the one given.
fn format_fraction(section: &Section, value: f64, locale: &DisplayLocale) -> String {
    let tokens = &section.tokens;
    let slash = tokens.iter().position(|token| *token == Token::Slash).expect("a fraction has a slash");
    let numerator_start = tokens[..slash].iter().rposition(|t| !is_placeholder(t)).map_or(0, |p| p + 1);
    let whole_placeholders: Vec<&Token> = tokens[..numerator_start].iter().filter(|t| is_placeholder(t)).collect();
    let denominator_end = slash
        + 1
        + tokens[slash + 1..].iter().take_while(|t| is_placeholder(t) || is_literal_digit(t)).count();
    let denominator_tokens = &tokens[slash + 1..denominator_end];
    let fixed: Option<u64> = if denominator_tokens.iter().any(is_literal_digit) {
        let digits: String = denominator_tokens
            .iter()
            .map(|token| match token {
                Token::Literal(text) => text.as_str(),
                _ => "0",
            })
            .collect();
        digits.parse().ok().filter(|&d| d > 0)
    } else {
        None
    };

    let has_whole = !whole_placeholders.is_empty();
    let (mut whole, part) = if has_whole { (value.trunc(), value.fract()) } else { (0.0, value) };
    let (mut numerator, denominator) = match fixed {
        Some(denominator) => ((part * denominator as f64).round() as u64, denominator),
        None => closest_fraction(part, 10u64.pow(denominator_tokens.len().clamp(1, 6) as u32) - 1),
    };
    if has_whole && numerator == denominator {
        whole += 1.0;
        numerator = 0;
    }

    let mut out = String::new();
    for token in &tokens[..numerator_start] {
        if !is_placeholder(token) {
            push_plain(&mut out, token);
        }
    }
    if has_whole {
        let first_placeholder = tokens.iter().position(is_placeholder).expect("whole placeholders");
        out.clear();
        for token in &tokens[..first_placeholder] {
            push_plain(&mut out, token);
        }
        if whole != 0.0 || numerator == 0 {
            out.push_str(&format_digits(
                &Section { tokens: whole_placeholders.iter().map(|t| (*t).clone()).collect(), condition: None, kind: Kind::Number },
                whole,
                locale,
            ));
        }
        if numerator == 0 {
            return out;
        }
        for token in &tokens[tokens[..numerator_start].iter().rposition(is_placeholder).map_or(0, |p| p + 1)..numerator_start] {
            push_plain(&mut out, token);
        }
    }
    out.push_str(&numerator.to_string());
    out.push('/');
    out.push_str(&denominator.to_string());
    for token in &tokens[denominator_end..] {
        push_plain(&mut out, token);
    }
    out
}

/// The fraction closest to a positive `value` with a denominator up to `limit`: the last
/// continued-fraction convergent that fits, or the best semiconvergent after it.
fn closest_fraction(value: f64, limit: u64) -> (u64, u64) {
    let (mut p0, mut q0, mut p1, mut q1) = (0u64, 1u64, 1u64, 0u64);
    let mut rest = value;
    loop {
        let whole = rest.floor();
        let a = whole as u64;
        let q2 = q0.saturating_add(a.saturating_mul(q1));
        if q2 > limit {
            break;
        }
        (p0, q0, p1, q1) = (p1, q1, p0 + a * p1, q2);
        let fraction = rest - whole;
        if fraction < 1e-9 {
            break;
        }
        rest = 1.0 / fraction;
    }
    if q1 == 0 {
        return (value.round() as u64, 1);
    }
    let k = (limit - q0) / q1;
    let semi = (p0 + k * p1, q0 + k * q1);
    let error = |(p, q): (u64, u64)| (value - p as f64 / q as f64).abs();
    if k > 0 && error(semi) < error((p1, q1)) { semi } else { (p1, q1) }
}

// MARK: Dates

#[derive(Debug)]
struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    /// 0 is Sunday.
    weekday: i64,
    hour: i64,
    minute: i64,
    second: i64,
    /// Thousandths of a second.
    millisecond: i64,
}

/// The largest serial Excel shows as a date: 9999-12-31.
const LAST_DATE: f64 = 2_958_465.999_999;

impl DateTime {
    /// The date and time of a serial number, rounded to the shown part of a second.
    fn from_serial(serial: f64, date1904: bool, subsecond_digits: usize) -> Option<Self> {
        if !(0.0..=LAST_DATE).contains(&serial) {
            return None;
        }
        let step = 10i64.pow(3 - subsecond_digits.min(3) as u32);
        let mut days = serial.floor() as i64;
        let mut milliseconds = (((serial - serial.floor()) * 86_400_000.0) / step as f64).round() as i64 * step;
        if milliseconds >= 86_400_000 {
            days += 1;
            milliseconds -= 86_400_000;
        }
        // Days since 1970-01-01. The 1900 system counts 1900-02-29, which never was, as day 60.
        let (year, month, day, unix_days) = if date1904 {
            let unix = days - 24_107;
            let (y, m, d) = civil_from_days(unix);
            (y, m, d, unix)
        } else if days == 0 {
            (1900, 1, 0, -25_568)
        } else if days == 60 {
            (1900, 2, 29, -25_508)
        } else {
            let unix = if days < 60 { days - 25_568 } else { days - 25_569 };
            let (y, m, d) = civil_from_days(unix);
            (y, m, d, unix)
        };
        Some(Self {
            year,
            month,
            day,
            weekday: (unix_days + 4).rem_euclid(7),
            hour: milliseconds / 3_600_000,
            minute: milliseconds / 60_000 % 60,
            second: milliseconds / 1000 % 60,
            millisecond: milliseconds % 1000,
        })
    }
}

fn subsecond_digits(section: &Section) -> usize {
    section
        .tokens
        .iter()
        .find_map(|token| match token {
            Token::SubSecond(digits) => Some(*digits),
            _ => None,
        })
        .unwrap_or(0)
}

fn format_date(section: &Section, date: &DateTime, serial: f64, locale: &DisplayLocale) -> String {
    let twelve_hours = section.tokens.iter().any(|token| matches!(token, Token::AmPm(..)));
    let name = |names: &[String], index: i64| names.get(index as usize).cloned().unwrap_or_default();
    let padded = |value: i64, width: usize| format!("{value:0width$}");
    let mut out = String::new();
    for token in &section.tokens {
        match *token {
            Token::Year(count) if count <= 2 => out.push_str(&padded(date.year % 100, 2)),
            Token::Year(_) => out.push_str(&padded(date.year, 4)),
            Token::Month(1) => out.push_str(&date.month.to_string()),
            Token::Month(2) => out.push_str(&padded(date.month, 2)),
            Token::Month(3) => out.push_str(&name(&locale.month_abbreviations, date.month - 1)),
            Token::Month(5) => out.extend(name(&locale.month_names, date.month - 1).chars().next()),
            Token::Month(_) => out.push_str(&name(&locale.month_names, date.month - 1)),
            Token::Day(1) => out.push_str(&date.day.to_string()),
            Token::Day(2) => out.push_str(&padded(date.day, 2)),
            Token::Day(3) => out.push_str(&name(&locale.day_abbreviations, date.weekday)),
            Token::Day(_) => out.push_str(&name(&locale.day_names, date.weekday)),
            Token::Hour(count) => {
                let hour = if twelve_hours {
                    match date.hour % 12 {
                        0 => 12,
                        hour => hour,
                    }
                } else {
                    date.hour
                };
                out.push_str(&padded(hour, count.min(2)));
            }
            Token::Minute(count) => out.push_str(&padded(date.minute, count.min(2))),
            Token::Second(count) => out.push_str(&padded(date.second, count.min(2))),
            Token::SubSecond(digits) => {
                out.push(locale.decimal_separator);
                let thousandths = format!("{:03}", date.millisecond);
                out.push_str(&thousandths[..digits.min(3)]);
            }
            Token::AmPm(full, upper) => {
                let morning = date.hour < 12;
                match (full, upper) {
                    (true, _) => out.push_str(if morning { &locale.am } else { &locale.pm }),
                    (false, true) => out.push(if morning { 'A' } else { 'P' }),
                    (false, false) => out.push(if morning { 'a' } else { 'p' }),
                }
            }
            Token::Elapsed(unit, count) => {
                let seconds = (serial * 86_400.0).round() as i64;
                let total = match unit {
                    Unit::Hours => seconds / 3600,
                    Unit::Minutes => seconds / 60,
                    Unit::Seconds => seconds,
                };
                out.push_str(&padded(total, count));
            }
            ref token => push_plain(&mut out, token),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(code: &str, value: f64) -> String {
        NumberFormat::parse(code).format_number(value, &DisplayLocale::default(), false)
    }

    fn russian() -> DisplayLocale {
        DisplayLocale {
            decimal_separator: ',',
            group_separator: '\u{a0}',
            short_date: "dd.mm.yyyy".to_owned(),
            ..DisplayLocale::default()
        }
    }

    #[test]
    fn general_numbers() {
        let locale = DisplayLocale::default();
        assert_eq!(general(0.0, &locale), "0");
        assert_eq!(general(42.0, &locale), "42");
        assert_eq!(general(-1250.5, &locale), "-1250.5");
        assert_eq!(general(0.1 + 0.2, &locale), "0.3");
        assert_eq!(general(0.000123, &locale), "0.000123");
        assert_eq!(general(123456789012345.0, &locale), "123456789012345");
        assert_eq!(general(1e20, &locale), "1E+20");
        assert_eq!(general(1.5e-10, &locale), "1.5E-10");
        assert_eq!(general(2.5, &russian()), "2,5");
    }

    #[test]
    fn digits_and_rounding() {
        assert_eq!(show("0", 2.5), "3");
        assert_eq!(show("0.00", 2.675), "2.68");
        assert_eq!(show("0.00", -0.001), "0.00");
        assert_eq!(show("#,##0", 1234567.0), "1,234,567");
        assert_eq!(show("#,##0.00", -1234.5), "-1,234.50");
        assert_eq!(show("#,##0.00", 0.5), "0.50");
        assert_eq!(show("#.##", 0.5), ".5");
        assert_eq!(show("0.0#", 1.5), "1.5");
        assert_eq!(show("0.0#", 1.25), "1.25");
        assert_eq!(show("000000", 42.0), "000042");
        assert_eq!(show("000-00-0000", 123456789.0), "123-45-6789");
        assert_eq!(show(".00", 12.5), "12.50");
        assert_eq!(show("#,##0,", 1234567.0), "1,235");
        assert_eq!(show("0.0,,\"M\"", 1234567.0), "1.2M");
        assert_eq!(
            NumberFormat::parse("#,##0.00").format_number(1234.5, &russian(), false),
            "1\u{a0}234,50"
        );
    }

    #[test]
    fn percent_and_scientific() {
        assert_eq!(show("0%", 0.256), "26%");
        assert_eq!(show("0.00%", 0.256), "25.60%");
        assert_eq!(show("0.00E+00", 12345.0), "1.23E+04");
        assert_eq!(show("0.00E+00", 0.00012), "1.20E-04");
        assert_eq!(show("0.0E+0", 9.99), "1.0E+1");
        assert_eq!(show("##0.0E+0", 12345.0), "12.3E+3");
        assert_eq!(show("0.00E+00", 0.0), "0.00E+00");
    }

    #[test]
    fn sections_literals_and_conditions() {
        assert_eq!(show("#,##0.00;(#,##0.00)", -5.0), "(5.00)");
        assert_eq!(show("0;-0;\"zero\"", 0.0), "zero");
        assert_eq!(show("0;;", 0.0), "");
        assert_eq!(show("[Red]0.0;[Blue]-0.0", -1.25), "-1.3");
        assert_eq!(show("\"$\"#,##0", 1500.0), "$1,500");
        assert_eq!(show("#,##0 [$€-407]", 1500.0), "1,500 €");
        assert_eq!(show("0 \\k\\g", 5.0), "5 kg");
        assert_eq!(show("[>=1000]#,##0,\"K\";0", 2500.0), "3K");
        assert_eq!(show("[>=1000]#,##0,\"K\";0", 999.0), "999");
        assert_eq!(show("General\" ₽\"", 99.5), "99.5 ₽");
        // The accounting format pads with spaces that would only get in the way.
        let accounting = "_-* #,##0.00\\ \"₽\"_-;\\-* #,##0.00\\ \"₽\"_-;_-* \"-\"??\\ \"₽\"_-;_-@_-";
        let format = NumberFormat::parse(accounting);
        assert_eq!(format.format_number(1234.5, &russian(), false), "1\u{a0}234,50 ₽");
        assert_eq!(format.format_number(-1234.5, &russian(), false), "-1\u{a0}234,50 ₽");
        assert_eq!(format.format_number(0.0, &russian(), false), "- ₽");
        assert_eq!(NumberFormat::parse("@").format_text("  x "), "  x ");
        assert_eq!(format.format_text("n/a"), "n/a");
    }

    #[test]
    fn text_sections() {
        let format = NumberFormat::parse("0.00;-0.00;0;\"[\"@\"]\"");
        assert_eq!(format.format_text("abc"), "[abc]");
        assert_eq!(NumberFormat::parse("@").format_number(1.5, &DisplayLocale::default(), false), "1.5");
        assert!(NumberFormat::parse("@").format_text("x") == "x");
    }

    #[test]
    fn fractions() {
        assert_eq!(show("# ?/?", 1.5), "1 1/2");
        assert_eq!(show("# ??/??", std::f64::consts::PI), "3 14/99");
        assert_eq!(show("# ???/???", std::f64::consts::PI), "3 16/113");
        assert_eq!(show("?/?", 0.75), "3/4");
        assert_eq!(show("# ?/8", 2.3), "2 2/8");
        assert_eq!(show("# ?/?", 2.0), "2");
        assert_eq!(show("# ?/?", -0.5), "-1/2");
    }

    #[test]
    fn dates_and_times() {
        // 45658 is 2025-01-01, a Wednesday.
        assert_eq!(show("yyyy-mm-dd", 45658.0), "2025-01-01");
        assert_eq!(show("m/d/yyyy", 45658.0), "1/1/2025");
        assert_eq!(show("d-mmm-yy", 45658.0), "1-Jan-25");
        assert_eq!(show("dddd, mmmm d", 45658.0), "Wednesday, January 1");
        assert_eq!(show("mmmmm", 45658.0), "J");
        assert_eq!(show("h:mm", 45658.75), "18:00");
        assert_eq!(show("h:mm AM/PM", 45658.75), "6:00 PM");
        assert_eq!(show("hh:mm:ss", 0.5 + 1.0 / 86400.0 * 0.6), "12:00:01");
        // Minutes are not rounded up by the seconds that are not shown.
        assert_eq!(show("h:mm", 10.0 / 24.0 + 30.0 / 1440.0 + 45.0 / 86400.0), "10:30");
        assert_eq!(show("mm:ss.0", 1.0 / 86400.0 * 61.3), "01:01.3");
        assert_eq!(show("[h]:mm", 1.5), "36:00");
        assert_eq!(show("[mm]:ss", 1.0 / 24.0), "60:00");
        assert_eq!(show("yyyy-mm-dd", 60.0), "1900-02-29");
        assert_eq!(show("yyyy-mm-dd", 61.0), "1900-03-01");
        assert_eq!(show("yyyy-mm-dd", 1.0), "1900-01-01");
        assert_eq!(NumberFormat::parse("yyyy-mm-dd").format_number(0.0, &DisplayLocale::default(), true), "1904-01-01");
        assert_eq!(show("yyyy-mm-dd", -1.0), "-1");
        let locale = russian();
        let short = builtin_code(14, &locale).unwrap();
        assert_eq!(NumberFormat::parse(&short).format_number(45658.5, &locale, false), "01.01.2025");
        let with_time = builtin_code(22, &locale).unwrap();
        assert_eq!(NumberFormat::parse(&with_time).format_number(45658.5, &locale, false), "01.01.2025 12:00");
    }
}
