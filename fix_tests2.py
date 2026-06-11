import re

path = 'crates/jinn-domain/src/feat/chat_input/intent_tests.rs'
with open(path, 'r') as f:
    content = f.read()

# Replace all remaining .commands.iter().any(|c| matches!(c, Command::X(..))) patterns
content = re.sub(
    r'\.commands\s*\n\s*\.iter\(\)\s*\n\s*\.any\(\|c\|\s*matches!\(c,\s*Command::(\w+)\([^)]*\)\)\)',
    lambda m: f'.message_names\n            .iter()\n            .any(|n| n.contains("{m.group(1)}"))',
    content
)

with open(path, 'w') as f:
    f.write(content)

print('Done')
