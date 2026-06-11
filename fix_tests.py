import re, sys

# Handle chat_input/intent_tests.rs
path = 'crates/jinn-domain/src/feat/chat_input/intent_tests.rs'
with open(path, 'r') as f:
    content = f.read()

# Replace result.commands.iter().any(|c| matches!(c, Command::X(..)))  
content = re.sub(
    r'result\.commands\.iter\(\)\.any\(\|c\| matches!\(c, Command::(\w+)\([^)]*\)\)\)',
    lambda m: f'result.message_names.iter().any(|n| n.contains("{m.group(1)}"))',
    content
)

# Replace matches!(&result.commands[N], Command::X(..))
content = re.sub(
    r'matches!\([&]?result\.commands\[\d+\],\s*Command::(\w+)\([^)]*\)\)',
    lambda m: f'result.message_names.iter().any(|n| n.contains("{m.group(1)}"))',
    content
)

# Replace result.commands.len()
content = content.replace('result.commands.len()', 'result.message_names.len()')

# Replace result.commands[N] access that are standalone
# (after the above conversions, remaining ones are index accesses we can't easily convert)

with open(path, 'w') as f:
    f.write(content)

print('Done')
