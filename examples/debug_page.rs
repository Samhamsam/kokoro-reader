fn main() {
    // Test abbreviation handling
    let test = "John D. Rockefeller said that Mr. Smith and Dr. Jones met at St. Patrick's Cathedral. They discussed U.S. policy.";
    let sentences = kokoro_reader::tts::split_into_sentences(test);
    println!("=== ABBREVIATION TEST ===");
    for (i, s) in sentences.iter().enumerate() {
        println!("[{}] {}", i, s);
    }
}
