# History Panel Pagination Spec

## ADDED Requirements

### Requirement: Version history panel supports paginated loading

The version history panel MUST allow operators to load additional version rows incrementally when a policy has more than 20 versions, rather than fetching all versions at once.

#### Scenario: Load more button appears when more versions exist
Given the operator opens the version history panel for a policy with more than 20 versions
When the initial 20 versions load
Then a "Load more" button appears below the version table

#### Scenario: Load more appends next page of versions
Given the "Load more" button is visible
When the operator clicks it
Then the next 20 versions are fetched and appended to the existing list
And the offset advances by 20

#### Scenario: Load more button disappears when all versions are loaded
Given the operator has clicked "Load more" enough times to exhaust all versions
When the last page returns fewer than 20 rows
Then the "Load more" button disappears

#### Scenario: Spinner shows while loading next page
Given the operator clicks "Load more"
When the fetch is in progress
Then a spinner appears inside the "Load more" button and the button is disabled
